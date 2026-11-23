//! `tetherd`: discovers devices on the LAN, pairs them with a short code,
//! keeps mutually authenticated QUIC connections to trusted devices, and syncs
//! clipboard text through the signed, replicated operation log.

#![forbid(unsafe_code)]

mod control;
mod ctx;
mod discovery;
mod input;
mod keystore;
mod net;
mod pair;
mod store;
mod tls;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{bail, Context, Result};
use clap::Parser;
use tether_core::{Clock, OpLog, SystemClock};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::ctx::Ctx;
use crate::input::Line;
use crate::tls::Gate;

#[derive(Parser, Debug)]
#[command(name = "tetherd", version, about = "Tether sync daemon")]
struct Args {
    /// Friendly name shown to other devices.
    #[arg(long, default_value_t = default_name())]
    name: String,

    /// Separate identity and data on one machine (for example, to run two
    /// daemons side by side). Letters, digits, '-' and '_'.
    #[arg(long, default_value = "default")]
    profile: String,

    /// Use a throwaway identity and keep nothing on disk (tests and demos).
    #[arg(long)]
    ephemeral: bool,

    /// UDP port for QUIC (0 picks a free one).
    #[arg(long, default_value_t = 0)]
    port: u16,

    /// Read clipboard text from stdin and print received text, instead of
    /// using the system clipboard. Use this to run two daemons on one machine.
    #[arg(long)]
    stdin: bool,

    /// Also dial this address (ip:port) of an already-paired device. Repeatable.
    #[arg(long)]
    peer: Vec<SocketAddr>,

    /// Turn off mDNS discovery (for networks that block multicast).
    #[arg(long)]
    no_mdns: bool,
}

fn default_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "device".to_string())
}

fn data_dir(profile: &str) -> Result<PathBuf> {
    let base = dirs::data_local_dir().context("could not find the local data folder")?;
    Ok(base.join("Tether").join(profile))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args = Args::parse();
    if !keystore::valid_profile(&args.profile) {
        bail!("--profile may only contain letters, digits, '-' and '_' (at most 32)");
    }

    // Identity and durable state.
    let (key, loaded) = if args.ephemeral {
        (keystore::generate()?, None)
    } else {
        let key = keystore::load_or_create(&args.profile)?;
        let db_key = keystore::load_or_create_db_key(&args.profile)?;
        let loaded = store::Store::open(&data_dir(&args.profile)?, &db_key)?;
        (key, Some(loaded))
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let mut log = OpLog::new(key.clone());
    let store = match loaded {
        Some(l) => {
            if let Some(aside) = &l.set_aside {
                warn!(
                    path = %aside.display(),
                    "the database was encrypted with a different key (was the credential store reset?); \
                     it was moved aside and this device starts empty. Pair it again to rejoin your devices."
                );
            }
            if l.imported > 0 {
                info!(ops = l.imported, "moved history from the old plain-text file into the encrypted database");
            }
            if let Some(root) = l.root {
                log.set_root(root);
            }
            let out = log.apply_batch(l.ops, clock.now_ms());
            if l.dropped > 0 || out.rejected > 0 {
                warn!(
                    dropped = l.dropped,
                    rejected = out.rejected,
                    "some stored operations were unreadable or failed verification and were ignored"
                );
            }
            Some(l.store)
        }
        None => None,
    };
    let tls_id = tls::identity(&key)?;
    drop(key);

    let (deliver_tx, deliver_rx) = std::sync::mpsc::channel();
    let gate = Arc::new(RwLock::new(Gate::default()));
    let ctx = Ctx::new(log, args.name.clone(), store, gate, tls_id, clock, deliver_tx);
    if ctx.lock().log.is_revoked() {
        warn!("this device was revoked by its group; it will not sync. Pair it again to rejoin.");
    }

    let endpoint = net::make_endpoint(&ctx, args.port)?;
    let port = endpoint.local_addr()?.port();
    info!(device = %ctx.me().to_hex(), name = %ctx.name(), port, members = ctx.lock().log.trust().members.len(), "tetherd started (type /help for commands)");

    let (local_tx, mut local_rx) = mpsc::unbounded_channel::<String>();
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<Line>();
    if args.stdin {
        input::spawn_stdout(deliver_rx);
    } else {
        input::spawn_clipboard(local_tx, deliver_rx);
    }
    input::spawn_stdin(line_tx, args.stdin);

    tokio::spawn(net::accept_loop(ctx.clone(), endpoint.clone()));
    tokio::spawn(net::reconnect_loop(ctx.clone(), endpoint.clone(), args.peer.clone()));
    let _mdns = if args.no_mdns { None } else { Some(discovery::start(ctx.clone(), port)?) };

    loop {
        tokio::select! {
            Some(text) = local_rx.recv() => copied(&ctx, text),
            Some(line) = line_rx.recv() => match line {
                Line::Command(cmd) => control::handle(&ctx, &endpoint, &cmd).await,
                Line::Text(text) => copied(&ctx, text),
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    info!("shutting down");
    endpoint.close(0u32.into(), b"shutdown");
    endpoint.wait_idle().await;
    Ok(())
}

/// A local copy: record it (unless it must stay local) and push it to peers.
fn copied(ctx: &Ctx, text: String) {
    if let Some(op) = ctx.local_copy(text) {
        net::broadcast(ctx, op);
    }
}
