//! The replicated operation log.
//!
//! Every device keeps every device's operations, each device's in `seq` order.
//! Peers sync by exchanging version vectors (the highest `seq` held per device)
//! and sending each other what is missing. Stage 2 moves this from memory to
//! SQLite and adds compaction.

use std::collections::BTreeMap;

use crate::{DeviceId, Hlc, Op, OpKind, Timestamp};

/// Highest contiguous `seq` held for each device.
pub type VersionVector = BTreeMap<DeviceId, u64>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    /// New operation, now part of the log.
    Applied,
    /// Already held. Safe to ignore: application is idempotent.
    Duplicate,
    /// Arrived before an earlier operation from the same device. The caller
    /// should pull again; the operation will be re-sent in order.
    Gap { expected: u64 },
}

#[derive(Debug, Clone)]
pub struct OpLog {
    me: DeviceId,
    hlc: Hlc,
    /// `ops[d][i]` has `seq == i + 1`.
    ops: BTreeMap<DeviceId, Vec<Op>>,
}

impl OpLog {
    pub fn new(me: DeviceId) -> Self {
        Self { me, hlc: Hlc::new(), ops: BTreeMap::new() }
    }

    pub fn me(&self) -> DeviceId {
        self.me
    }

    pub fn clock(&self) -> Timestamp {
        self.hlc.last()
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

    /// Record a local change and return the operation to broadcast.
    pub fn append_local(&mut self, kind: OpKind, physical_ms: u64) -> Op {
        let ts = self.hlc.tick(physical_ms);
        let me = self.me;
        let entry = self.ops.entry(me).or_default();
        let op = Op { device: me, seq: entry.len() as u64 + 1, ts, kind };
        entry.push(op.clone());
        op
    }

    /// Apply an operation received from a peer.
    pub fn apply_remote(&mut self, op: Op, physical_ms: u64) -> ApplyResult {
        let have = self.ops_from(op.device).len() as u64;
        if op.seq <= have {
            return ApplyResult::Duplicate;
        }
        if op.seq != have + 1 {
            return ApplyResult::Gap { expected: have + 1 };
        }
        self.hlc.observe(op.ts, physical_ms);
        self.ops.entry(op.device).or_default().push(op);
        ApplyResult::Applied
    }

    pub fn version_vector(&self) -> VersionVector {
        self.ops.iter().map(|(d, ops)| (*d, ops.len() as u64)).collect()
    }

    /// Operations a peer with version vector `theirs` does not have yet,
    /// each device's in `seq` order.
    pub fn missing_for(&self, theirs: &VersionVector) -> Vec<Op> {
        self.ops
            .iter()
            .flat_map(|(d, ops)| {
                let have = theirs.get(d).copied().unwrap_or(0) as usize;
                ops.iter().skip(have).cloned()
            })
            .collect()
    }

    /// Clipboard history, newest first. Identical on every device that holds
    /// the same operations, whatever order they arrived in.
    pub fn clipboard_history(&self) -> Vec<&Op> {
        let mut items: Vec<&Op> =
            self.ops.values().flatten().filter(|op| matches!(op.kind, OpKind::ClipboardText(_))).collect();
        items.sort_by_key(|op| std::cmp::Reverse(op.order_key()));
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> OpKind {
        OpKind::ClipboardText(s.to_string())
    }

    #[test]
    fn remote_apply_is_idempotent() {
        let (a, b) = (DeviceId::from_u128(1), DeviceId::from_u128(2));
        let mut log_a = OpLog::new(a);
        let mut log_b = OpLog::new(b);
        let op = log_a.append_local(text("hello"), 1_000);
        assert_eq!(log_b.apply_remote(op.clone(), 1_000), ApplyResult::Applied);
        assert_eq!(log_b.apply_remote(op, 1_000), ApplyResult::Duplicate);
        assert_eq!(log_b.len(), 1);
    }

    #[test]
    fn out_of_order_operations_are_reported_as_gaps() {
        let mut log_a = OpLog::new(DeviceId::from_u128(1));
        let mut log_b = OpLog::new(DeviceId::from_u128(2));
        let _first = log_a.append_local(text("one"), 1_000);
        let second = log_a.append_local(text("two"), 1_001);
        assert_eq!(log_b.apply_remote(second, 1_001), ApplyResult::Gap { expected: 1 });
        assert!(log_b.is_empty());
    }

    #[test]
    fn missing_for_sends_only_what_the_peer_lacks() {
        let mut log_a = OpLog::new(DeviceId::from_u128(1));
        let mut log_b = OpLog::new(DeviceId::from_u128(2));
        for i in 0..5 {
            log_a.append_local(text(&format!("item {i}")), 1_000 + i);
        }
        for op in log_a.missing_for(&log_b.version_vector()).into_iter().take(2) {
            log_b.apply_remote(op, 2_000);
        }
        let rest = log_a.missing_for(&log_b.version_vector());
        assert_eq!(rest.iter().map(|op| op.seq).collect::<Vec<_>>(), vec![3, 4, 5]);
    }

    #[test]
    fn history_order_is_the_same_on_every_device() {
        let (a, b) = (DeviceId::from_u128(1), DeviceId::from_u128(2));
        let mut log_a = OpLog::new(a);
        let mut log_b = OpLog::new(b);
        // Concurrent copies at the same wall time on both devices.
        let op_a = log_a.append_local(text("from a"), 1_000);
        let op_b = log_b.append_local(text("from b"), 1_000);
        log_a.apply_remote(op_b, 1_001);
        log_b.apply_remote(op_a, 1_001);
        let ids = |log: &OpLog| log.clipboard_history().iter().map(|op| op.id()).collect::<Vec<_>>();
        assert_eq!(ids(&log_a), ids(&log_b));
    }
}
