//! The pairing ceremony over QUIC (protocol in `tether_core::pairing`).
//!
//! A pairing connection is authenticated only by TLS keys that are not yet
//! trusted, so it may carry nothing but pairing messages, must finish within
//! a deadline, and is closed afterwards. Sync starts on a fresh connection
//! once both sides trust each other.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use quinn::{Connection, Endpoint};
use tether_core::pairing::{is_valid_code, Binding, Inviter, Joiner, EXPORTER_LABEL};
use tether_core::{DeviceId, Message};
use tokio::time::timeout;
use tracing::{info, warn};

use crate::ctx::Ctx;
use crate::net::{self, HANDSHAKE_TIMEOUT};
use crate::tls::{self, Expect};

const PAIRING_DEADLINE: Duration = Duration::from_secs(30);

fn channel_binding(conn: &Connection) -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, b"")
        .map_err(|_| anyhow!("could not derive the channel binding"))?;
    Ok(out)
}

async fn read(conn: &Connection) -> Result<(quinn::SendStream, Message)> {
    let (send, mut recv) = conn.accept_bi().await?;
    let bytes = recv.read_to_end(Message::MAX_FRAME).await?;
    let msg = Message::decode(&bytes)?;
    if !msg.is_pairing() {
        bail!("only pairing messages are allowed before pairing");
    }
    Ok((send, msg))
}

async fn reply(mut send: quinn::SendStream, msg: &Message) -> Result<()> {
    send.write_all(&msg.encode()).await?;
    send.finish()?;
    Ok(())
}

/// Inviter side: an unknown device connected while our pairing window is open.
pub async fn serve_inviter(ctx: Ctx, conn: Connection, joiner: DeviceId) {
    match timeout(PAIRING_DEADLINE, inviter_flow(&ctx, &conn, joiner)).await {
        Ok(Ok(())) => info!(device = %joiner, "paired"),
        Ok(Err(e)) => warn!(device = %joiner, error = %e, "pairing failed"),
        Err(_) => warn!(device = %joiner, "pairing timed out"),
    }
    // Let the last reply drain before closing.
    tokio::time::sleep(Duration::from_millis(200)).await;
    conn.close(0u32.into(), b"pairing finished");
}

async fn inviter_flow(ctx: &Ctx, conn: &Connection, joiner: DeviceId) -> Result<()> {
    let channel = channel_binding(conn)?;

    let (send, msg) = read(conn).await?;
    let Message::PairStart { spake } = msg else {
        reply(send, &Message::PairFailed).await?;
        bail!("expected PairStart");
    };
    // Counts as one attempt whether or not the code turns out to be right.
    let Some(code) = ctx.begin_pair_attempt() else {
        reply(send, &Message::PairFailed).await?;
        bail!("no pairing window is open");
    };
    let binding = Binding { inviter: ctx.me(), joiner, channel };
    let Ok((inviter, spake_i, confirm)) = Inviter::respond(binding, &code, &spake) else {
        reply(send, &Message::PairFailed).await?;
        bail!("invalid pairing message");
    };
    let root = ctx.lock().log.root();
    reply(send, &Message::PairReply { spake: spake_i, confirm, root, name: ctx.name() }).await?;

    let (send, msg) = read(conn).await?;
    let Message::PairConfirm { confirm, name } = msg else {
        reply(send, &Message::PairFailed).await?;
        bail!("expected PairConfirm");
    };
    if inviter.verify(&confirm).is_err() {
        reply(send, &Message::PairFailed).await?;
        bail!("the other device did not know the code");
    }
    let op = ctx.pairing_succeeded(joiner, &name)?;
    reply(send, &Message::PairDone).await?;
    net::broadcast(ctx, op);
    Ok(())
}

/// Joiner side: try each candidate address until one accepts the code.
pub async fn join(ctx: Ctx, endpoint: Endpoint, code: String, candidates: Vec<SocketAddr>) -> Result<DeviceId> {
    if !is_valid_code(&code) {
        bail!("a pairing code is six digits");
    }
    if candidates.is_empty() {
        bail!("no devices found nearby; give the other device's address, for example /join {code} 192.168.1.20:5000");
    }
    for addr in candidates {
        match try_join(&ctx, &endpoint, &code, addr).await {
            Ok(inviter) => {
                let (ctx, endpoint) = (ctx.clone(), endpoint.clone());
                tokio::spawn(async move {
                    if let Err(e) = net::connect(ctx, endpoint, addr, Expect::Device(inviter)).await {
                        warn!(%addr, error = %e, "could not connect after pairing");
                    }
                });
                return Ok(inviter);
            }
            Err(e) => warn!(%addr, error = %e, "pairing attempt failed"),
        }
    }
    bail!("no device accepted the code")
}

async fn try_join(ctx: &Ctx, endpoint: &Endpoint, code: &str, addr: SocketAddr) -> Result<DeviceId> {
    let config = tls::client_config(&ctx.tls, ctx.gate.clone(), Expect::Pairing)?;
    let conn = timeout(HANDSHAKE_TIMEOUT, endpoint.connect_with(config, addr, "tether")?)
        .await
        .map_err(|_| anyhow!("handshake timed out"))?
        .context("connecting")?;
    let result = timeout(PAIRING_DEADLINE, async {
        let inviter = tls::peer_id(&conn).ok_or_else(|| anyhow!("no device key"))?;
        if inviter == ctx.me() {
            bail!("that is this device");
        }
        let binding = Binding { inviter, joiner: ctx.me(), channel: channel_binding(&conn)? };
        let (joiner, spake_j) = Joiner::start(binding, code);
        let Message::PairReply { spake, confirm, root, name } =
            net::request(&conn, &Message::PairStart { spake: spake_j }).await?
        else {
            bail!("the other device refused");
        };
        let mine = joiner.finish(&spake, &confirm).map_err(|_| anyhow!("the code did not match"))?;
        match net::request(&conn, &Message::PairConfirm { confirm: mine, name: ctx.name() }).await? {
            Message::PairDone => {
                ctx.joined(inviter, root, &name)?;
                Ok(inviter)
            }
            _ => bail!("the other device refused"),
        }
    })
    .await
    .map_err(|_| anyhow!("pairing timed out"))?;
    conn.close(0u32.into(), b"pairing finished");
    result
}
