//! QUIC transport. Each request is one bi-directional stream: write a
//! `Message`, finish, read the reply. Streams are independent, so a slow
//! request never blocks another (no head-of-line blocking).
//!
//! Every connection is mutually authenticated by device key before any
//! message is read (see `tls`). On top of that: a cap on connections, a rate
//! limit on streams per connection, timeouts on every read, and strict
//! decoding of every message.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use quinn::{Connection, Endpoint, RecvStream, SendStream};
use tether_core::{Message, Op};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::ctx::Ctx;
use crate::pair;
use crate::tls::{self, Expect};

/// Most simultaneous connections; beyond this new ones are refused.
const MAX_CONNECTIONS: usize = 32;
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// A peer has this long to send a whole request, or to answer one.
const STREAM_TIMEOUT: Duration = Duration::from_secs(15);
/// Streams a peer may open per second before it is disconnected.
const MAX_STREAMS_PER_SECOND: u32 = 64;
/// Upper bound on pages in one catch-up, so a misbehaving peer cannot keep us pulling forever.
const MAX_PULL_PAGES: usize = 10_000;

/// One endpoint both accepts and dials, on a single UDP port.
pub fn make_endpoint(ctx: &Ctx, port: u16) -> Result<Endpoint> {
    let server = tls::server_config(&ctx.tls, ctx.gate.clone())?;
    let mut endpoint = Endpoint::server(server, SocketAddr::from(([0, 0, 0, 0], port)))?;
    endpoint.set_default_client_config(tls::client_config(&ctx.tls, ctx.gate.clone(), Expect::AnyTrusted)?);
    Ok(endpoint)
}

pub async fn accept_loop(ctx: Ctx, endpoint: Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        if endpoint.open_connections() >= MAX_CONNECTIONS {
            debug!(addr = %incoming.remote_address(), "refusing connection: too many open");
            incoming.refuse();
            continue;
        }
        let (ctx, endpoint) = (ctx.clone(), endpoint.clone());
        tokio::spawn(async move {
            match timeout(HANDSHAKE_TIMEOUT, incoming).await {
                Ok(Ok(conn)) => on_incoming(ctx, endpoint, conn).await,
                // Strangers fail here: the TLS verifier rejected their key.
                Ok(Err(e)) => debug!(error = %e, "handshake refused"),
                Err(_) => debug!("handshake timed out"),
            }
        });
    }
}

async fn on_incoming(ctx: Ctx, endpoint: Endpoint, conn: Connection) {
    let Some(peer) = tls::peer_id(&conn) else {
        conn.close(1u32.into(), b"no device key");
        return;
    };
    if ctx.is_trusted(peer) {
        info!(%peer, addr = %conn.remote_address(), "accepted connection");
        run_session(ctx, endpoint, conn, peer).await;
    } else if ctx.gate.read().map(|g| g.pairing_open()).unwrap_or(false) {
        info!(%peer, "pairing attempt");
        pair::serve_inviter(ctx, conn, peer).await;
    } else {
        conn.close(1u32.into(), b"not trusted");
    }
}

/// Dial a trusted peer and sync with it until the connection closes.
pub async fn connect(ctx: Ctx, endpoint: Endpoint, addr: SocketAddr, expect: Expect) -> Result<()> {
    let config = tls::client_config(&ctx.tls, ctx.gate.clone(), expect)?;
    let conn = timeout(HANDSHAKE_TIMEOUT, endpoint.connect_with(config, addr, "tether")?)
        .await
        .map_err(|_| anyhow!("handshake timed out"))?
        .with_context(|| format!("connecting to {addr}"))?;
    let peer = tls::peer_id(&conn).ok_or_else(|| anyhow!("peer presented no device key"))?;
    if !ctx.is_trusted(peer) {
        conn.close(1u32.into(), b"not trusted");
        bail!("{addr} is not a trusted device");
    }
    info!(%peer, %addr, "connected");
    run_session(ctx, endpoint, conn, peer).await;
    Ok(())
}

/// Serve one authenticated connection until it closes.
async fn run_session(ctx: Ctx, _endpoint: Endpoint, conn: Connection, peer: tether_core::DeviceId) {
    if let Some(old) = ctx.lock().peers.insert(peer, conn.clone()) {
        old.close(0u32.into(), b"replaced");
    }

    // Catch up on anything missed while apart.
    {
        let (ctx, conn) = (ctx.clone(), conn.clone());
        tokio::spawn(async move {
            if let Err(e) = pull(&ctx, &conn).await {
                warn!(%peer, error = %e, "initial sync failed");
            }
        });
    }

    let (mut window_start, mut in_window) = (Instant::now(), 0u32);
    loop {
        match conn.accept_bi().await {
            Ok((send, recv)) => {
                if window_start.elapsed() >= Duration::from_secs(1) {
                    (window_start, in_window) = (Instant::now(), 0);
                }
                in_window += 1;
                if in_window > MAX_STREAMS_PER_SECOND {
                    warn!(%peer, "too many requests; disconnecting");
                    conn.close(2u32.into(), b"rate limited");
                    break;
                }
                let (ctx, conn) = (ctx.clone(), conn.clone());
                tokio::spawn(async move {
                    if let Err(e) = handle_stream(&ctx, &conn, peer, send, recv).await {
                        debug!(%peer, error = %e, "request failed");
                    }
                });
            }
            Err(e) => {
                info!(%peer, reason = %e, "connection closed");
                break;
            }
        }
    }

    let id = conn.stable_id();
    let mut s = ctx.lock();
    if s.peers.get(&peer).is_some_and(|c| c.stable_id() == id) {
        s.peers.remove(&peer);
    }
}

async fn read_message(recv: &mut RecvStream) -> Result<Message> {
    let bytes = timeout(STREAM_TIMEOUT, recv.read_to_end(Message::MAX_FRAME))
        .await
        .map_err(|_| anyhow!("request timed out"))??;
    Ok(Message::decode(&bytes)?)
}

async fn write_message(send: &mut SendStream, msg: &Message) -> Result<()> {
    timeout(STREAM_TIMEOUT, send.write_all(&msg.encode())).await.map_err(|_| anyhow!("reply timed out"))??;
    send.finish()?;
    Ok(())
}

async fn handle_stream(
    ctx: &Ctx,
    conn: &Connection,
    peer: tether_core::DeviceId,
    mut send: SendStream,
    mut recv: RecvStream,
) -> Result<()> {
    let msg = read_message(&mut recv).await?;
    // Trust can be withdrawn mid-session (revocation).
    if !ctx.is_trusted(peer) {
        conn.close(1u32.into(), b"not trusted");
        bail!("{peer} is no longer trusted");
    }
    let reply = match msg {
        Message::Hello { name, vv } => {
            ctx.note_name(peer, &name);
            let (ops, more) = ctx.missing_for(&vv);
            Message::Ops { ops, more }
        }
        Message::Ops { ops, .. } => {
            let outcome = ctx.apply(ops);
            if outcome.pull_again {
                let (ctx, conn) = (ctx.clone(), conn.clone());
                tokio::spawn(async move {
                    if let Err(e) = pull(&ctx, &conn).await {
                        debug!(error = %e, "gap repair failed");
                    }
                });
            }
            Message::Ack { applied: outcome.applied as u32 }
        }
        other => bail!("unexpected message on a sync connection: {:?}", std::mem::discriminant(&other)),
    };
    write_message(&mut send, &reply).await
}

/// Send one message and wait for its reply.
pub async fn request(conn: &Connection, msg: &Message) -> Result<Message> {
    let (mut send, mut recv) =
        timeout(STREAM_TIMEOUT, conn.open_bi()).await.map_err(|_| anyhow!("could not open a stream"))??;
    write_message(&mut send, msg).await?;
    read_message(&mut recv).await
}

/// Send our version vector and apply what the peer sends back, page by page,
/// until we are caught up or a page brings nothing new.
pub async fn pull(ctx: &Ctx, conn: &Connection) -> Result<()> {
    for _ in 0..MAX_PULL_PAGES {
        match request(conn, &ctx.hello()).await? {
            Message::Ops { ops, more } => {
                let outcome = ctx.apply(ops);
                if !more || outcome.applied == 0 {
                    return Ok(());
                }
            }
            _ => bail!("unexpected reply to Hello"),
        }
    }
    Ok(())
}

/// Push a new local operation to every connected peer.
pub fn broadcast(ctx: &Ctx, op: Op) {
    let peers = ctx.peers();
    info!(seq = op.seq, peers = peers.len(), "broadcasting");
    for (peer, conn) in peers {
        let msg = Message::Ops { ops: vec![op.clone()], more: false };
        let seq = op.seq;
        tokio::spawn(async move {
            let started = Instant::now();
            match request(&conn, &msg).await {
                Ok(Message::Ack { applied }) => {
                    let rtt_ms = started.elapsed().as_secs_f64() * 1000.0;
                    info!(%peer, seq, applied, rtt_ms = format_args!("{rtt_ms:.2}"), "delivered");
                }
                Ok(_) => warn!(%peer, "unexpected reply"),
                Err(e) => warn!(%peer, error = %e, "send failed; the peer will catch up on reconnect"),
            }
        });
    }
}

/// Keep trusted nearby devices connected: dial any we are not connected to.
/// The lower device id dials, so a pair never opens two connections.
pub async fn reconnect_loop(ctx: Ctx, endpoint: Endpoint, explicit: Vec<SocketAddr>) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        let targets: Vec<(Option<tether_core::DeviceId>, SocketAddr)> = {
            let mut s = ctx.lock();
            let me = s.log.me();
            let mut t: Vec<_> = Vec::new();
            let nearby: Vec<_> = s.nearby.iter().map(|(d, a)| (*d, *a)).collect();
            for (device, addr) in nearby {
                let trusted = ctx.gate.read().map(|g| g.trusted.contains(&device)).unwrap_or(false);
                if trusted && me < device && !s.peers.contains_key(&device) && s.dialing.insert(device) {
                    t.push((Some(device), addr));
                }
            }
            if s.peers.is_empty() {
                t.extend(explicit.iter().map(|a| (None, *a)));
            }
            t
        };
        for (device, addr) in targets {
            let (ctx, endpoint) = (ctx.clone(), endpoint.clone());
            tokio::spawn(async move {
                let expect = device.map(Expect::Device).unwrap_or(Expect::AnyTrusted);
                if let Err(e) = connect(ctx.clone(), endpoint, addr, expect).await {
                    debug!(%addr, error = %e, "could not connect");
                }
                if let Some(d) = device {
                    ctx.lock().dialing.remove(&d);
                }
            });
        }
    }
}
