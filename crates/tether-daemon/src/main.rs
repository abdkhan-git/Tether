//! `tetherd`: discovers peers on the LAN, keeps QUIC connections open and
//! syncs clipboard text through the replicated operation log.

mod ctx;
mod discovery;
mod input;
mod net;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use tether_core::{Clock, DeviceId, Message, SystemClock};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::ctx::Ctx;

#[derive(Parser, Debug)]
#[command(name = "tetherd", version, about = "Tether sync daemon (stage 1)")]
struct Args {
    /// Friendly name shown to peers.
    #[arg(long, default_value_t = default_name())]
    name: String,

    /// UDP port for QUIC (0 picks a free one).
    #[arg(long, default_value_t = 0)]
    port: u16,

    /// Read lines from stdin and print received text, instead of using the
    /// system clipboard. Use this to run two daemons on one machine.
    #[arg(long)]
    stdin: bool,

    /// Also dial this peer directly (ip:port). Repeatable.
    #[arg(long)]
    peer: Vec<SocketAddr>,

    /// Turn off mDNS discovery (for networks that block multicast).
    #[arg(long)]
    no_mdns: bool,
}

fn default_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "device".to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args = Args::parse();
    let me = DeviceId::random();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);

    let (deliver_tx, deliver_rx) = std::sync::mpsc::channel();
    let ctx = Ctx::new(me, args.name.clone(), clock, deliver_tx);

    let endpoint = net::make_endpoint(args.port)?;
    let port = endpoint.local_addr()?.port();
    info!(device = %me, name = %args.name, port, "tetherd started");

    let (local_tx, mut local_rx) = mpsc::unbounded_channel::<String>();
    if args.stdin {
        input::spawn_stdin(local_tx, deliver_rx);
    } else {
        input::spawn_clipboard(local_tx, deliver_rx);
    }

    tokio::spawn(net::accept_loop(ctx.clone(), endpoint.clone()));

    let _mdns = if args.no_mdns { None } else { Some(discovery::start(ctx.clone(), endpoint.clone(), port)?) };

    for addr in args.peer {
        let (ctx, endpoint) = (ctx.clone(), endpoint.clone());
        tokio::spawn(async move {
            if let Err(e) = net::connect(ctx, endpoint, addr).await {
                warn!(%addr, error = %e, "could not connect");
            }
        });
    }

    loop {
        tokio::select! {
            Some(text) = local_rx.recv() => broadcast(&ctx, text),
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    info!("shutting down");
    endpoint.close(0u32.into(), b"shutdown");
    endpoint.wait_idle().await;
    Ok(())
}

/// Append a local copy to the log and push it to every connected peer.
fn broadcast(ctx: &Ctx, text: String) {
    let op = ctx.local_copy(text);
    let peers = ctx.peers();
    info!(seq = op.seq, peers = peers.len(), "local copy");
    for (peer, conn) in peers {
        let msg = Message::Ops(vec![op.clone()]);
        let seq = op.seq;
        tokio::spawn(async move {
            let started = Instant::now();
            match net::request(&conn, &msg).await {
                Ok(Message::Ack { applied }) => {
                    let rtt_ms = started.elapsed().as_secs_f64() * 1000.0;
                    info!(%peer, seq, applied, rtt_ms = format_args!("{rtt_ms:.2}"), "delivered");
                }
                Ok(other) => warn!(%peer, ?other, "unexpected reply"),
                Err(e) => warn!(%peer, error = %e, "send failed; the peer will catch up on reconnect"),
            }
        });
    }
}
