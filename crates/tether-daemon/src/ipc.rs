//! The daemon's end of the local control channel. Threat model:
//! docs/SECURITY.md, "Local control channel".
//!
//! The OS lets only this user reach the endpoint (`tether_os::local`). Each
//! connection must then complete the mutual token handshake within 5 s, or
//! it is closed having received nothing but a nonce. After that it sends
//! requests, one at a time, or subscribes to events.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use tether_ipc::auth::server_handshake;
use tether_ipc::frame::{recv, send};
use tether_ipc::{
    Endpoint, Event, Listener, Reply, Request, Response, Stream, Token, MAX_REPLY_BYTES, MAX_REQUEST_BYTES,
};
use tokio::io::{AsyncReadExt, AsyncWrite};
use tokio::sync::{broadcast, Semaphore};
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::api;
use crate::ctx::Ctx;
use crate::net;
use crate::notes::Edited;

/// Most control clients connected at once; more are closed on arrival.
pub const MAX_CLIENTS: usize = 8;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// A client that stops reading is dropped after this long.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Take the endpoint. Done before the token is published, so a second
/// daemon on the same profile fails here without replacing the first one's token.
pub fn bind(endpoint: &Endpoint) -> Result<Listener> {
    Listener::bind(endpoint).map_err(|e| anyhow!("cannot open the control channel at {endpoint}: {e}"))
}

pub async fn serve(mut listener: Listener, token: Token, ctx: Ctx, quic: quinn::Endpoint) {
    let token = Arc::new(token);
    let slots = Arc::new(Semaphore::new(MAX_CLIENTS));
    loop {
        let stream = match listener.accept().await {
            Ok(stream) => stream,
            Err(e) => {
                warn!(error = %e, "control channel accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            debug!("refusing a control client: {MAX_CLIENTS} already connected");
            continue; // dropping the stream closes it
        };
        let (token, ctx, quic) = (token.clone(), ctx.clone(), quic.clone());
        tokio::spawn(async move {
            if let Err(e) = session(stream, &token, &ctx, &quic).await {
                debug!(error = %e, "control client closed");
            }
            drop(slot);
        });
    }
}

async fn session(mut stream: Stream, token: &Token, ctx: &Ctx, quic: &quinn::Endpoint) -> Result<()> {
    timeout(HANDSHAKE_TIMEOUT, server_handshake(&mut stream, token))
        .await
        .map_err(|_| anyhow!("handshake timed out"))??;
    debug!("control client authenticated");
    loop {
        let request: Request = match recv(&mut stream, MAX_REQUEST_BYTES).await {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()), // malformed or oversized: close
        };
        if request == Request::Subscribe {
            // Subscribe before confirming, so no event falls in between.
            let events = ctx.events.subscribe();
            write(&mut stream, &Response::Ok(Reply::Subscribed)).await?;
            return stream_events(stream, events).await;
        }
        let response = match handle(ctx, quic, request).await {
            Ok(reply) => Response::Ok(reply),
            Err(e) => Response::Err(format!("{e:#}")),
        };
        write(&mut stream, &response).await?;
    }
}

async fn write<W: AsyncWrite + Unpin + ?Sized, T: serde::Serialize>(w: &mut W, msg: &T) -> Result<()> {
    timeout(WRITE_TIMEOUT, send(w, msg, MAX_REPLY_BYTES)).await.map_err(|_| anyhow!("client stopped reading"))??;
    Ok(())
}

async fn handle(ctx: &Ctx, quic: &quinn::Endpoint, request: Request) -> Result<Reply> {
    Ok(match request {
        Request::Status => Reply::Status(api::status(ctx, quic.local_addr()?.port())),
        Request::Devices => Reply::Devices(api::devices(ctx)),
        Request::History { before, limit } => Reply::History(api::history(ctx, before, limit)),
        Request::Pair => Reply::PairCode(api::pair(ctx)?),
        Request::Join { code, addr } => Reply::Joined { device: api::join(ctx, quic, code, addr).await? },
        Request::Revoke { device } => {
            api::revoke(ctx, device)?;
            Reply::Revoked
        }
        Request::SetPaused { paused } => Reply::Paused(api::set_paused(ctx, paused)?),
        Request::SetPinned { author, seq, pinned } => {
            api::set_pinned(ctx, (author, seq), pinned)?;
            Reply::Marked
        }
        Request::CopyHere { author, seq } => {
            ctx.copy_here((author, seq))?;
            Reply::Copied
        }
        Request::Send { author, seq, to } => {
            api::send(ctx, (author, seq), to)?;
            Reply::Sent
        }
        Request::Rename { device, name } => {
            api::rename(ctx, device, &name)?;
            Reply::Renamed
        }
        Request::Delete { author, seq } => {
            api::delete(ctx, (author, seq))?;
            Reply::Marked
        }
        Request::Watch => {
            ctx.watch();
            Reply::Watching
        }
        Request::Wake { device } => Reply::Woken { adapters: ctx.wake(device)? },
        Request::Notes => Reply::Notes(ctx.notes()),
        Request::NoteCreate { title, notebook } => {
            let (note, ops) = ctx.note_create(&title, &notebook)?;
            net::broadcast_all(ctx, ops);
            Reply::NoteCreated { note }
        }
        Request::NoteOpen { note, editor } => Reply::NoteText(ctx.note_open(note, editor)?),
        Request::NoteEdit(edit) => {
            ctx.note_turn().await;
            match ctx.note_edit(edit) {
                Ok(Edited::Done(ops)) => {
                    net::broadcast_all(ctx, ops);
                    Reply::NoteEdited
                }
                Ok(Edited::Stale) => Reply::NoteStale,
                Ok(Edited::Secret(kind)) => Reply::NoteSecret { kind: kind.to_string() },
                Err(e) => Reply::NoteRefused { reason: format!("{e:#}") },
            }
        }
        Request::NoteMeta { note, title, notebook, pinned, trashed } => {
            net::broadcast_all(ctx, ctx.note_meta(note, title, notebook, pinned, trashed)?);
            Reply::NoteChanged
        }
        Request::NoteHistory { note } => Reply::NoteHistory(ctx.note_history(note)?),
        Request::NoteAt { note, at, author } => Reply::NoteText(ctx.note_at(note, at, author)?),
        Request::NoteRestore { note, at, author } => {
            net::broadcast_all(ctx, ctx.note_restore(note, at, author)?);
            Reply::NoteChanged
        }
        Request::Subscribe => bail!("Subscribe is handled before this point"),
    })
}

async fn stream_events(stream: Stream, mut events: broadcast::Receiver<Event>) -> Result<()> {
    let (mut rd, mut wr) = tokio::io::split(stream);
    loop {
        let event = tokio::select! {
            ev = events.recv() => match ev {
                Ok(ev) => ev,
                Err(broadcast::error::RecvError::Lagged(missed)) => Event::Lagged { missed },
                Err(broadcast::error::RecvError::Closed) => return Ok(()),
            },
            // A subscriber sends nothing more; a byte or end-of-stream ends it.
            _ = rd.read_u8() => return Ok(()),
        };
        write(&mut wr, &event).await?;
    }
}
