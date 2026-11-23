//! What the daemon can be asked to do. The console (`control.rs`) and the
//! local control channel (`ipc.rs`) both call these, so the two can never
//! disagree about what a command means.

use std::net::SocketAddr;

use anyhow::{anyhow, Result};
use quinn::Endpoint;
use tether_core::content::{self, Delivery};
use tether_core::pairing::{code_from_random, MAX_ATTEMPTS, WINDOW_SECS};
use tether_core::{DeviceId, Op, OpKind};
use tether_ipc::{
    Cursor, Device, DeviceState, HistoryItem, HistoryPage, PairCode, Status, HISTORY_PAGE_BYTES, MAX_HISTORY_PAGE,
};

use crate::ctx::Ctx;
use crate::{net, pair};

/// Rough per-item overhead when budgeting a history page.
const ITEM_OVERHEAD: usize = 128;

/// The history entry for `op`, or `None` if it is not clipboard text.
pub fn item(op: &Op, me: DeviceId) -> Option<HistoryItem> {
    let OpKind::ClipboardText(text) = &op.kind else { return None };
    let held = match (op.device == me, content::delivery_for(text)) {
        (false, Delivery::Hold(reason)) => Some(reason),
        _ => None,
    };
    Some(HistoryItem { author: op.device, seq: op.seq, at: op.ts, text: text.clone(), held })
}

pub fn status(ctx: &Ctx, port: u16) -> Status {
    let pairing_open = ctx.gate.read().map(|g| g.pairing_open()).unwrap_or(false);
    let s = ctx.lock();
    Status {
        device: s.log.me(),
        name: s.name.clone(),
        root: s.log.root(),
        members: s.log.trust().members.len() as u32,
        connected: s.peers.len() as u32,
        revoked: s.log.is_revoked(),
        port,
        pairing_open,
        paused: s.paused,
    }
}

/// Pause or resume sync on this device.
pub fn set_paused(ctx: &Ctx, paused: bool) -> Result<bool> {
    ctx.set_paused(paused)?;
    Ok(paused)
}

/// Group members first (this device, then the others), then unpaired devices seen nearby.
pub fn devices(ctx: &Ctx) -> Vec<Device> {
    let s = ctx.lock();
    let me = s.log.me();
    let trust = s.log.trust();
    let mut out: Vec<Device> = trust
        .members
        .iter()
        .map(|(id, added_as)| {
            let state = if *id == me {
                DeviceState::ThisDevice
            } else if trust.cutoffs.contains_key(id) {
                DeviceState::Revoked
            } else if s.peers.contains_key(id) {
                DeviceState::Connected
            } else {
                DeviceState::Offline
            };
            let name =
                if *id == me { s.name.clone() } else { s.names.get(id).cloned().unwrap_or_else(|| added_as.clone()) };
            let rtt_us = s.peers.get(id).map(|c| u32::try_from(c.rtt().as_micros()).unwrap_or(u32::MAX));
            Device { id: *id, name, state, rtt_us, ops: s.log.ops_from(*id).len() as u64 }
        })
        .collect();
    out.sort_by_key(|d| d.state != DeviceState::ThisDevice);
    out.extend(s.nearby.keys().filter(|id| !trust.members.contains_key(id) && **id != me).map(|id| Device {
        id: *id,
        name: s.names.get(id).cloned().unwrap_or_default(),
        state: DeviceState::Nearby,
        rtt_us: None,
        ops: 0,
    }));
    out
}

/// A page of clipboard history, newest first, strictly older than `before`.
pub fn history(ctx: &Ctx, before: Option<Cursor>, limit: u16) -> HistoryPage {
    let limit = usize::from(limit.clamp(1, MAX_HISTORY_PAGE));
    let s = ctx.lock();
    let me = s.log.me();
    let all = s.log.clipboard_history();
    // `all` is sorted newest first, so the items at or after the cursor form a prefix.
    let start = before.map_or(0, |c| all.partition_point(|op| op.order_key() >= (c.at, c.device)));
    let (mut items, mut bytes) = (Vec::new(), 0usize);
    for op in &all[start..] {
        let Some(item) = item(op, me) else { continue };
        let size = item.text.len() + ITEM_OVERHEAD;
        if items.len() == limit || (!items.is_empty() && bytes + size > HISTORY_PAGE_BYTES) {
            break;
        }
        bytes += size;
        items.push(item);
    }
    let next = if start + items.len() < all.len() { items.last().map(HistoryItem::cursor) } else { None };
    HistoryPage { items, next }
}

/// A uniformly random six-digit code (rejection sampling avoids modulo bias).
fn random_code() -> Result<String> {
    loop {
        let n = getrandom::u32().map_err(|e| anyhow!("no secure randomness available: {e}"))?;
        if n < 4_294_000_000 {
            return Ok(code_from_random(n));
        }
    }
}

/// Open a pairing window and return its code.
pub fn pair(ctx: &Ctx) -> Result<PairCode> {
    let code = random_code()?;
    ctx.open_pairing(code.clone());
    Ok(PairCode { code, valid_secs: WINDOW_SECS as u32, attempts: MAX_ATTEMPTS })
}

/// Join the device showing `code`: at `addr` if given, else any unpaired device nearby.
pub async fn join(ctx: &Ctx, endpoint: &Endpoint, code: String, addr: Option<SocketAddr>) -> Result<DeviceId> {
    let candidates: Vec<SocketAddr> = match addr {
        Some(addr) => vec![addr],
        None => {
            let trusted = ctx.gate.read().map(|g| g.trusted.clone()).unwrap_or_default();
            ctx.lock().nearby.iter().filter(|(d, _)| !trusted.contains(d)).map(|(_, a)| *a).collect()
        }
    };
    pair::join(ctx.clone(), endpoint.clone(), code, candidates).await
}

/// Remove a member from the group and tell the other devices.
pub fn revoke(ctx: &Ctx, device: DeviceId) -> Result<()> {
    let op = ctx.revoke(device)?;
    net::broadcast(ctx, op);
    Ok(())
}
