//! Shared daemon state. The lock is a plain `std::sync::Mutex` and is never
//! held across an `.await`: every critical section is a few map operations
//! plus, until the SQLite move, one small fsync'd append.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use quinn::Connection;
use tether_core::content::{self, Delivery, HoldReason};
use tether_core::limits::MAX_NAME_BYTES;
use tether_core::pairing::{MAX_ATTEMPTS, WINDOW_SECS};
use tether_core::{Clock, DeviceId, LocalError, Message, Op, OpKind, OpLog, VersionVector};
use tracing::{error, info, warn};

use crate::store::Store;
use crate::tls::{SharedGate, TlsIdentity};

/// What the daemon hands to the clipboard (or console) thread.
#[derive(Debug, Clone)]
pub enum Delivered {
    /// Put this on the system clipboard.
    Text(String),
    /// Arrived from a peer but held in history, not applied (see `content`).
    Held { from: DeviceId, reason: HoldReason, preview: String },
    /// Copied here, looks like a secret, kept local and not synced.
    KeptLocal(&'static str),
}

pub struct PairWindow {
    pub code: String,
    until: Instant,
    attempts: u32,
}

impl PairWindow {
    fn is_open(&self) -> bool {
        Instant::now() < self.until && self.attempts < MAX_ATTEMPTS
    }
}

pub struct State {
    pub log: OpLog,
    pub name: String,
    /// Open connections, keyed by the peer's device key (from its certificate).
    pub peers: HashMap<DeviceId, Connection>,
    /// Peers we are currently dialing, so repeated discovery events don't open
    /// duplicate connections.
    pub dialing: HashSet<DeviceId>,
    /// Devices seen on the LAN, trusted or not.
    pub nearby: HashMap<DeviceId, SocketAddr>,
    /// Names peers report about themselves (display only, never trusted).
    pub names: HashMap<DeviceId, String>,
    pub pairing: Option<PairWindow>,
    /// Trusted because we just paired with them, before the log confirms it.
    pub pinned: HashSet<DeviceId>,
    store: Option<Store>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ApplyOutcome {
    pub applied: usize,
    /// Something arrived out of order or from a not-yet-known author.
    pub pull_again: bool,
}

#[derive(Clone)]
pub struct Ctx {
    state: Arc<Mutex<State>>,
    pub gate: SharedGate,
    pub tls: Arc<TlsIdentity>,
    pub clock: Arc<dyn Clock>,
    deliver: Sender<Delivered>,
}

/// Strip control and direction-changing characters from a name a peer chose,
/// and bound its length. Names are shown to the user, never trusted.
pub fn clean_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().filter(|c| !c.is_control() && !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
    {
        if out.len() + c.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        out.push(c);
    }
    out
}

fn preview(text: &str) -> String {
    let one_line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(60).collect();
    one_line.trim().to_string()
}

impl Ctx {
    pub fn new(
        log: OpLog,
        name: String,
        store: Option<Store>,
        gate: SharedGate,
        tls: TlsIdentity,
        clock: Arc<dyn Clock>,
        deliver: Sender<Delivered>,
    ) -> Self {
        let state = State {
            log,
            name: clean_name(&name),
            peers: HashMap::new(),
            dialing: HashSet::new(),
            nearby: HashMap::new(),
            names: HashMap::new(),
            pairing: None,
            pinned: HashSet::new(),
            store,
        };
        let ctx = Self { state: Arc::new(Mutex::new(state)), gate, tls: Arc::new(tls), clock, deliver };
        ctx.refresh_gate();
        ctx
    }

    /// The lock, recovering from poisoning: a panic elsewhere must not take
    /// the daemon down with it.
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn me(&self) -> DeviceId {
        self.lock().log.me()
    }

    pub fn name(&self) -> String {
        self.lock().name.clone()
    }

    pub fn peers(&self) -> Vec<(DeviceId, Connection)> {
        self.lock().peers.iter().map(|(id, c)| (*id, c.clone())).collect()
    }

    pub fn is_trusted(&self, device: DeviceId) -> bool {
        self.gate.read().map(|g| g.trusted.contains(&device)).unwrap_or(false)
    }

    pub fn hello(&self) -> Message {
        let s = self.lock();
        Message::Hello { name: s.name.clone(), vv: s.log.version_vector() }
    }

    pub fn missing_for(&self, vv: &VersionVector) -> (Vec<Op>, bool) {
        self.lock().log.missing_for(vv)
    }

    pub fn note_name(&self, peer: DeviceId, name: &str) {
        self.lock().names.insert(peer, clean_name(name));
    }

    fn persist(s: &mut State, op: &Op) {
        if let Some(store) = s.store.as_mut() {
            if let Err(e) = store.append(op) {
                error!(error = %e, "could not persist an operation; it will be re-fetched from peers");
            }
        }
    }

    /// Record something copied on this device. Returns the operation to
    /// broadcast, or `None` if it must not leave this device.
    pub fn local_copy(&self, text: String) -> Option<Op> {
        if let Some(kind) = content::secret_kind(&text) {
            let _ = self.deliver.send(Delivered::KeptLocal(kind));
            return None;
        }
        let now = self.clock.now_ms();
        let mut s = self.lock();
        if s.log.is_revoked() {
            return None;
        }
        match s.log.append_local(OpKind::ClipboardText(text), now) {
            Ok(op) => {
                Self::persist(&mut s, &op);
                Some(op)
            }
            Err(LocalError::TooLarge) => {
                warn!("copied text is larger than the sync limit; not synced");
                None
            }
            Err(e) => {
                warn!(?e, "could not record a local copy");
                None
            }
        }
    }

    /// Apply operations from a peer, persist them, and surface the newest
    /// clipboard item according to the delivery policy.
    pub fn apply(&self, ops: Vec<Op>) -> ApplyOutcome {
        let now = self.clock.now_ms();
        let (out, delivery, changed) = {
            let mut s = self.lock();
            let before = s.log.version_vector();
            let out = s.log.apply_batch(ops, now);
            if out.membership_changed {
                let ops: Vec<Op> = s.log.all_ops().cloned().collect();
                if let Some(store) = s.store.as_mut() {
                    if let Err(e) = store.rewrite(ops.iter()) {
                        error!(error = %e, "could not rewrite the operation log");
                    }
                }
            } else if out.applied > 0 {
                let fresh: Vec<Op> = s
                    .log
                    .version_vector()
                    .into_iter()
                    .flat_map(|(author, held)| {
                        let had = before.get(&author).copied().unwrap_or(0) as usize;
                        s.log.ops_from(author)[had.min(held as usize)..].to_vec()
                    })
                    .collect();
                for op in &fresh {
                    Self::persist(&mut s, op);
                }
            }
            for device in &out.equivocators {
                error!(%device, "device signed two different operations with the same sequence number; its key may be compromised");
            }
            // Surface the newest item only if it just arrived from a peer.
            let me = s.log.me();
            let delivery = s.log.clipboard_history().first().and_then(|op| {
                let new = op.seq > before.get(&op.device).copied().unwrap_or(0);
                match (&op.kind, new && op.device != me) {
                    (OpKind::ClipboardText(text), true) => Some(match content::delivery_for(text) {
                        Delivery::Apply => Delivered::Text(text.clone()),
                        Delivery::Hold(reason) => Delivered::Held { from: op.device, reason, preview: preview(text) },
                    }),
                    _ => None,
                }
            });
            let changed = out.membership_changed;
            (out, delivery, changed)
        };
        if let Some(d) = delivery {
            let _ = self.deliver.send(d);
        }
        if changed {
            self.refresh_gate();
        }
        ApplyOutcome { applied: out.applied, pull_again: out.gap || out.unknown_author > 0 }
    }

    /// Recompute who may connect, and close connections to anyone who may not.
    pub fn refresh_gate(&self) {
        let (trusted, pairing_until, to_close) = {
            let mut s = self.lock();
            let me = s.log.me();
            let trusted: HashSet<DeviceId> = if s.log.is_revoked() {
                HashSet::new()
            } else {
                let trust = s.log.trust();
                let mut t: HashSet<DeviceId> = trust.members.keys().filter(|d| trust.is_active(**d)).copied().collect();
                t.extend(s.pinned.iter().filter(|d| !trust.cutoffs.contains_key(d)).copied());
                t.remove(&me);
                t
            };
            if s.pairing.as_ref().is_some_and(|w| !w.is_open()) {
                s.pairing = None;
            }
            let pairing_until = s.pairing.as_ref().map(|w| w.until);
            let to_close: Vec<(DeviceId, Connection)> =
                s.peers.iter().filter(|(d, _)| !trusted.contains(d)).map(|(d, c)| (*d, c.clone())).collect();
            for (d, _) in &to_close {
                s.peers.remove(d);
            }
            (trusted, pairing_until, to_close)
        };
        if let Ok(mut g) = self.gate.write() {
            g.trusted = trusted;
            g.pairing_until = pairing_until;
        }
        for (device, conn) in to_close {
            info!(%device, "closing connection: device is no longer trusted");
            conn.close(1u32.into(), b"not trusted");
        }
    }

    /// Open a pairing window with this code.
    pub fn open_pairing(&self, code: String) {
        self.lock().pairing =
            Some(PairWindow { code, until: Instant::now() + Duration::from_secs(WINDOW_SECS), attempts: 0 });
        self.refresh_gate();
    }

    /// Spend one attempt from the pairing window. Each SPAKE2 run is one
    /// guess, so it is counted before the guess is checked.
    pub fn begin_pair_attempt(&self) -> Option<String> {
        let code = {
            let mut s = self.lock();
            match s.pairing.as_mut() {
                Some(w) if w.is_open() => {
                    w.attempts += 1;
                    Some(w.code.clone())
                }
                _ => None,
            }
        };
        self.refresh_gate();
        code
    }

    /// Inviter side: the joiner proved it knows the code. Add it to the group.
    pub fn pairing_succeeded(&self, joiner: DeviceId, name: &str) -> Result<Op> {
        let now = self.clock.now_ms();
        let op = {
            let mut s = self.lock();
            let op = s
                .log
                .append_local(OpKind::DeviceAdded { device: joiner, name: clean_name(name) }, now)
                .map_err(|e| anyhow!("{e:?}"))?;
            Self::persist(&mut s, &op);
            s.pairing = None; // one device per code
            s.names.insert(joiner, clean_name(name));
            op
        };
        self.refresh_gate();
        Ok(op)
    }

    /// Joiner side: the inviter proved it knows the code. Join its group.
    pub fn joined(&self, inviter: DeviceId, root: DeviceId, name: &str) -> Result<()> {
        {
            let mut s = self.lock();
            s.log.set_root(root);
            if let Some(store) = s.store.as_mut() {
                store.set_root(root)?;
            }
            s.pinned.insert(inviter);
            s.names.insert(inviter, clean_name(name));
        }
        self.refresh_gate();
        Ok(())
    }

    /// Revoke a member. Its later operations are rejected by every device.
    pub fn revoke(&self, device: DeviceId) -> Result<Op> {
        let now = self.clock.now_ms();
        let op = {
            let mut s = self.lock();
            if device == s.log.me() {
                bail!("a device cannot revoke itself here; revoke it from another device");
            }
            if !s.log.trust().is_active(device) {
                bail!("that device is not an active member");
            }
            let after_seq = s.log.ops_from(device).len() as u64;
            let op =
                s.log.append_local(OpKind::DeviceRevoked { device, after_seq }, now).map_err(|e| anyhow!("{e:?}"))?;
            let ops: Vec<Op> = s.log.all_ops().cloned().collect();
            if let Some(store) = s.store.as_mut() {
                store.rewrite(ops.iter())?;
            }
            s.pinned.remove(&device);
            op
        };
        self.refresh_gate();
        Ok(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_bounded() {
        assert_eq!(clean_name("Laptop\u{1b}[31m\u{202E}x"), "Laptop[31mx");
        assert!(clean_name(&"é".repeat(100)).len() <= MAX_NAME_BYTES);
    }
}
