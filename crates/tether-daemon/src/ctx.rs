//! Shared daemon state. The lock is a plain `std::sync::Mutex` and is never
//! held across an `.await`: every critical section is a few map operations
//! plus at most one small database transaction.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use quinn::Connection;
use tether_core::content::{self, Delivery, HoldReason};
use tether_core::limits::MAX_NAME_BYTES;
use tether_core::pairing::{MAX_ATTEMPTS, WINDOW_SECS};
use tether_core::{Clock, DeviceId, ItemId, LocalError, Message, Op, OpKind, OpLog, VersionVector};
use tether_ipc::Event;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::api;
use crate::store::Store;
use crate::tls::{SharedGate, TlsIdentity};

/// Events queued per control-channel subscriber before it is told it lagged.
const EVENT_QUEUE: usize = 256;
/// More new history items than this in one batch become one `HistoryChanged`.
const MAX_ITEM_EVENTS: usize = 16;

/// What the daemon hands to the clipboard (or console) thread.
#[derive(Debug, Clone)]
pub enum Delivered {
    /// Put this on the system clipboard.
    Text(String),
    /// Arrived from a peer but held in history, not applied (see `content`).
    Held { from: DeviceId, reason: HoldReason, preview: String },
    /// Copied here, looks like a secret, kept local and not synced.
    KeptLocal(&'static str),
    /// Copied here, marked private by the app that copied it; kept local.
    MarkedPrivate,
}

/// A change to a history item (see [`Ctx::mark`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Pin(bool),
    Delete,
}

/// Hands deliveries to whoever shows them (the clipboard thread, or stdout).
pub type Deliver = Arc<dyn Fn(Delivered) + Send + Sync>;

/// What `Event::KeptLocal` says about a copy its app marked private.
pub const MARKED_PRIVATE: &str = "copy its app marked private";

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
    /// This device's display name: its latest rename in the log, or `default_name`.
    pub name: String,
    /// The name given on the command line (`--name`), used until a rename.
    default_name: String,
    /// Open connections, keyed by the peer's device key (from its certificate).
    pub peers: HashMap<DeviceId, Connection>,
    /// Devices seen on the LAN, trusted or not.
    pub nearby: HashMap<DeviceId, SocketAddr>,
    /// Names peers report about themselves (display only, never trusted).
    pub names: HashMap<DeviceId, String>,
    pub pairing: Option<PairWindow>,
    /// Trusted because we just paired with them, before the log confirms it.
    pub pinned: HashSet<DeviceId>,
    /// Sync is paused: local copies are not recorded, and peers' copies are
    /// not put on the system clipboard (they still arrive in history).
    pub paused: bool,
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
    deliver: Deliver,
    /// Fans state changes out to control-channel subscribers.
    pub events: broadcast::Sender<Event>,
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
        deliver: Deliver,
    ) -> Self {
        let state = State {
            log,
            name: clean_name(&name),
            default_name: clean_name(&name),
            peers: HashMap::new(),
            nearby: HashMap::new(),
            names: HashMap::new(),
            pairing: None,
            pinned: HashSet::new(),
            paused: false,
            store,
        };
        let (events, _) = broadcast::channel(EVENT_QUEUE);
        let ctx = Self { state: Arc::new(Mutex::new(state)), gate, tls: Arc::new(tls), clock, deliver, events };
        Self::refresh_own_name(&mut ctx.lock());
        ctx.refresh_gate();
        ctx
    }

    /// Tell control-channel subscribers (if any) that something changed.
    pub fn emit(&self, event: Event) {
        let _ = self.events.send(event);
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
    /// broadcast, or `None` if it must not leave this device: the app that
    /// copied it marked it `private`, or it looks like a secret.
    pub fn local_copy(&self, text: String, private: bool) -> Option<Op> {
        if self.lock().paused {
            return None;
        }
        if private {
            (self.deliver)(Delivered::MarkedPrivate);
            self.emit(Event::KeptLocal { kind: MARKED_PRIVATE.to_string() });
            return None;
        }
        if let Some(kind) = content::secret_kind(&text) {
            (self.deliver)(Delivered::KeptLocal(kind));
            self.emit(Event::KeptLocal { kind: kind.to_string() });
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
                let me = s.log.me();
                drop(s);
                if let Some(item) = api::item(&op, me, false) {
                    self.emit(Event::Clipboard(item));
                }
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
        let (out, delivery, changed, new_items, item_changes, renamed, received) = {
            let mut s = self.lock();
            let before = s.log.version_vector();
            let out = s.log.apply_batch(ops, now);
            let fresh: Vec<Op> = if out.applied == 0 {
                Vec::new()
            } else {
                s.log
                    .version_vector()
                    .into_iter()
                    .flat_map(|(author, held)| {
                        let had = before.get(&author).copied().unwrap_or(0) as usize;
                        s.log.ops_from(author)[had.min(held as usize)..].to_vec()
                    })
                    .collect()
            };
            if out.membership_changed {
                let ops: Vec<Op> = s.log.all_ops().cloned().collect();
                if let Some(store) = s.store.as_mut() {
                    if let Err(e) = store.rewrite(ops.iter()) {
                        error!(error = %e, "could not rewrite the operation log");
                    }
                }
            } else if !fresh.is_empty() {
                if let Some(store) = s.store.as_mut() {
                    if let Err(e) = store.append_all(&fresh) {
                        error!(error = %e, "could not persist operations; they will be re-fetched from peers");
                    }
                }
            }
            let me = s.log.me();
            let trust = s.log.trust();
            let marks = s.log.marks();
            let mut new_items: Vec<_> = fresh
                .iter()
                // The same rule as `OpLog::clipboard_history`.
                .filter(|op| op.device == me || trust.accepts(op.device, op.seq))
                .filter(|op| !marks.deleted.contains(&op.id()))
                .filter_map(|op| api::item(op, me, marks.pinned.contains(&op.id())))
                .collect();
            new_items.sort_by_key(|item| (item.at, item.author));
            // Pins and deletes that arrived: say where each item stands now.
            let targets: BTreeSet<ItemId> = fresh
                .iter()
                .filter_map(|op| match &op.kind {
                    OpKind::Pin { item, .. } | OpKind::Delete { item } => Some((item.device, item.seq)),
                    _ => None,
                })
                .collect();
            let item_changes: Vec<Event> = targets
                .into_iter()
                .map(|(author, seq)| Event::ItemChanged {
                    author,
                    seq,
                    pinned: marks.pinned.contains(&(author, seq)),
                    deleted: marks.deleted.contains(&(author, seq)),
                })
                .collect();
            for device in &out.equivocators {
                error!(%device, "device signed two different operations with the same sequence number; its key may be compromised");
            }
            // Surface the newest item only if it just arrived from a peer, and
            // never while paused.
            let me = s.log.me();
            let newest = s.log.clipboard_history().first().filter(|_| !s.paused).and_then(|op| {
                let new = op.seq > before.get(&op.device).copied().unwrap_or(0);
                match (&op.kind, new && op.device != me) {
                    (OpKind::ClipboardText(text), true) => Some(match content::delivery_for(text) {
                        Delivery::Apply => Delivered::Text(text.clone()),
                        Delivery::Hold(reason) => Delivered::Held { from: op.device, reason, preview: preview(text) },
                    }),
                    _ => None,
                }
            });
            // A device sent us an item on purpose: that wins over the newest
            // copy. Only fresh sends, never while paused, and the content
            // policy still applies (a command stays held).
            let sent = fresh.iter().rev().filter(|_| !s.paused).find_map(|op| {
                let text = s.log.send_for_me(op).filter(|_| tether_core::send_is_fresh(op.ts.wall_ms, now))?;
                let OpKind::Send { item, .. } = &op.kind else { return None };
                let delivered = match content::delivery_for(text) {
                    Delivery::Apply => Delivered::Text(text.to_string()),
                    Delivery::Hold(reason) => Delivered::Held { from: op.device, reason, preview: preview(text) },
                };
                let event = Event::Received {
                    from: op.device,
                    author: item.device,
                    seq: item.seq,
                    held: matches!(delivered, Delivered::Held { .. }),
                };
                Some((delivered, event))
            });
            let (delivery, received) = match sent {
                Some((delivered, event)) => (Some(delivered), Some(event)),
                None => (newest, None),
            };
            let changed = out.membership_changed;
            let renamed = fresh.iter().any(|op| matches!(op.kind, OpKind::Rename { .. })) || changed;
            if renamed {
                Self::refresh_own_name(&mut s);
            }
            (out, delivery, changed, new_items, item_changes, renamed, received)
        };
        if let Some(d) = delivery {
            (self.deliver)(d);
        }
        if let Some(event) = received {
            self.emit(event);
        }
        for event in item_changes {
            self.emit(event);
        }
        if new_items.len() > MAX_ITEM_EVENTS {
            self.emit(Event::HistoryChanged);
        } else {
            for item in new_items {
                self.emit(Event::Clipboard(item));
            }
        }
        if changed {
            self.refresh_gate(); // also tells subscribers that devices changed
        } else if renamed {
            self.emit(Event::DevicesChanged);
        }
        ApplyOutcome { applied: out.applied, pull_again: out.gap || out.unknown_author > 0 }
    }

    /// This device's name follows its latest rename in the log, cleaned like
    /// any name; without one (or if the rename was pruned), the `--name` one.
    fn refresh_own_name(s: &mut State) {
        let me = s.log.me();
        s.name = s.log.renames().get(&me).map(|n| clean_name(n)).unwrap_or_else(|| s.default_name.clone());
    }

    /// Put history item `item` on this device's clipboard, as the user asked in
    /// the app or the palette. It goes through the clipboard thread, which
    /// knows its own writes, so it is not recorded again as a new copy. An
    /// explicit local action: allowed while paused, and for held items (the
    /// user chose to use it, SECURITY.md rule 3).
    pub fn copy_here(&self, item: ItemId) -> Result<()> {
        let text = self.lock().log.history_text(item).map(str::to_string);
        let text = text.ok_or_else(|| anyhow!("that item is not in history"))?;
        (self.deliver)(Delivered::Text(text));
        Ok(())
    }

    /// Ask device `to` to put history item `item` on its clipboard. Returns the
    /// operation to broadcast. `to` must be another active member, and the
    /// item must be in history (held, from a member, not deleted).
    pub fn send(&self, item: ItemId, to: DeviceId) -> Result<Op> {
        let now = self.clock.now_ms();
        let mut s = self.lock();
        if s.log.is_revoked() {
            bail!("this device was removed from its group");
        }
        if to == s.log.me() {
            bail!("that is this device; copy the item here instead");
        }
        if !s.log.trust().is_active(to) {
            bail!("that device is not an active member");
        }
        let target = s.log.item_ref(item).ok_or_else(|| anyhow!("that item is not in history"))?;
        let op = s.log.append_local(OpKind::Send { item: target, to }, now).map_err(|e| anyhow!("{e:?}"))?;
        Self::persist(&mut s, &op);
        Ok(op)
    }

    /// Give a member a display name on every device. Returns the operation to
    /// broadcast. The name is cleaned (no control or direction characters,
    /// bounded) before it is signed.
    pub fn rename(&self, device: DeviceId, name: &str) -> Result<Op> {
        let name = clean_name(name.trim());
        if name.is_empty() {
            bail!("a device name cannot be empty");
        }
        let now = self.clock.now_ms();
        let op = {
            let mut s = self.lock();
            if s.log.is_revoked() {
                bail!("this device was removed from its group");
            }
            if !s.log.trust().is_active(device) {
                bail!("that device is not an active member");
            }
            let op = s.log.append_local(OpKind::Rename { device, name }, now).map_err(|e| anyhow!("{e:?}"))?;
            Self::persist(&mut s, &op);
            Self::refresh_own_name(&mut s);
            op
        };
        self.emit(Event::DevicesChanged);
        Ok(op)
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
        self.emit(Event::DevicesChanged);
    }

    /// Pin, unpin or delete a history item, as an operation of this device.
    /// Returns the operation to broadcast. The item must be in history (held,
    /// from a member, not deleted); the operation names it by content hash.
    pub fn mark(&self, item: ItemId, mark: Mark) -> Result<Op> {
        let now = self.clock.now_ms();
        let (op, event) = {
            let mut s = self.lock();
            if s.log.is_revoked() {
                bail!("this device was removed from its group");
            }
            let target = s.log.item_ref(item).ok_or_else(|| anyhow!("that item is not in history"))?;
            let kind = match mark {
                Mark::Pin(pinned) => OpKind::Pin { item: target, pinned },
                Mark::Delete => OpKind::Delete { item: target },
            };
            let op = s.log.append_local(kind, now).map_err(|e| anyhow!("{e:?}"))?;
            Self::persist(&mut s, &op);
            let marks = s.log.marks();
            let event = Event::ItemChanged {
                author: item.0,
                seq: item.1,
                pinned: marks.pinned.contains(&item),
                deleted: marks.deleted.contains(&item),
            };
            (op, event)
        };
        self.emit(event);
        Ok(op)
    }

    /// Pause or resume sync, and remember the choice across restarts. If it
    /// cannot be saved, nothing changes.
    pub fn set_paused(&self, paused: bool) -> Result<()> {
        {
            let mut s = self.lock();
            if s.paused == paused {
                return Ok(());
            }
            if let Some(store) = s.store.as_mut() {
                store.set_paused(paused)?;
            }
            s.paused = paused;
        }
        info!(paused, "sync {}", if paused { "paused" } else { "resumed" });
        self.emit(Event::Paused(paused));
        Ok(())
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

    use std::sync::RwLock;
    use tether_core::{DeviceKey, SystemClock};

    #[test]
    fn names_are_cleaned_and_bounded() {
        assert_eq!(clean_name("Laptop\u{1b}[31m\u{202E}x"), "Laptop[31mx");
        assert!(clean_name(&"é".repeat(100)).len() <= MAX_NAME_BYTES);
    }

    /// A daemon context with no network, recording what it delivers.
    fn ctx() -> (Ctx, Arc<Mutex<Vec<Delivered>>>) {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let key = DeviceKey::for_test(1);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let deliver: Deliver = Arc::new(move |d| sink.lock().unwrap().push(d));
        let tls = crate::tls::identity(&key).unwrap();
        let gate = Arc::new(RwLock::new(crate::tls::Gate::default()));
        (Ctx::new(OpLog::new(key), "test".into(), None, gate, tls, Arc::new(SystemClock), deliver), seen)
    }

    #[test]
    fn a_copy_marked_private_is_kept_local_and_not_logged() {
        let (ctx, seen) = ctx();
        let mut events = ctx.events.subscribe();
        assert!(ctx.local_copy("hunter2-but-not-pattern-shaped".into(), true).is_none());
        assert!(ctx.lock().log.is_empty(), "nothing was logged");
        assert!(matches!(seen.lock().unwrap().as_slice(), [Delivered::MarkedPrivate]));
        assert_eq!(events.try_recv().unwrap(), Event::KeptLocal { kind: MARKED_PRIVATE.into() });

        // The same text without the mark is an ordinary copy.
        assert!(ctx.local_copy("hunter2-but-not-pattern-shaped".into(), false).is_some());
        assert_eq!(ctx.lock().log.len(), 1);
    }

    #[test]
    fn while_paused_nothing_is_recorded_or_applied_and_history_still_fills() {
        let (ctx, seen) = ctx();
        let peer_key = DeviceKey::for_test(2);
        let now = ctx.clock.now_ms();
        ctx.lock().log.append_local(OpKind::DeviceAdded { device: peer_key.id(), name: "peer".into() }, now).unwrap();
        let mut peer = OpLog::new(peer_key);
        peer.set_root(ctx.me());
        let mut events = ctx.events.subscribe();

        ctx.set_paused(true).unwrap();
        assert_eq!(events.try_recv().unwrap(), Event::Paused(true));
        let before = ctx.lock().log.len();
        assert!(ctx.local_copy("copied while paused".into(), false).is_none());
        assert_eq!(ctx.lock().log.len(), before, "a local copy is not recorded while paused");

        let op = peer.append_local(OpKind::ClipboardText("from the peer".into()), now + 1).unwrap();
        assert_eq!(ctx.apply(vec![op]).applied, 1, "a peer's copy still arrives");
        assert!(seen.lock().unwrap().is_empty(), "nothing reaches the system clipboard while paused");
        assert!(matches!(events.try_recv().unwrap(), Event::Clipboard(item) if item.text == "from the peer"));

        ctx.set_paused(false).unwrap();
        assert_eq!(events.try_recv().unwrap(), Event::Paused(false));
        let op = peer.append_local(OpKind::ClipboardText("second".into()), now + 2).unwrap();
        ctx.apply(vec![op]);
        assert!(matches!(seen.lock().unwrap().as_slice(), [Delivered::Text(t)] if t == "second"));
        assert!(ctx.local_copy("copied after resuming".into(), false).is_some());
    }
}
