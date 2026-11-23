//! Hard limits on everything a peer can send. Every decoded message is checked
//! against these before any of it is used (SECURITY.md, rule 6).

use crate::{Op, OpKind};

/// Largest clipboard text that syncs. Bigger content belongs to file transfer.
pub const MAX_TEXT_BYTES: usize = 1 << 20;
/// Longest device name.
pub const MAX_NAME_BYTES: usize = 64;
/// Most operations in one message; a peer with more pages through them.
pub const MAX_OPS_PER_MESSAGE: usize = 256;
/// Approximate byte budget for one page of operations.
pub const MAX_PAGE_BYTES: usize = 2 << 20;
/// Most devices a version vector may describe.
pub const MAX_VV_ENTRIES: usize = 1024;
/// Largest encoded message accepted from the network.
pub const MAX_FRAME: usize = 4 << 20;
/// SPAKE2 messages on Ed25519 are exactly 33 bytes.
pub const SPAKE_MSG_BYTES: usize = 33;

/// Fixed per-operation overhead used when budgeting pages.
const OP_OVERHEAD: usize = 160;

/// Rough encoded size of an operation, for paging.
pub fn op_size(op: &Op) -> usize {
    OP_OVERHEAD
        + match &op.kind {
            OpKind::ClipboardText(t) => t.len(),
            OpKind::DeviceAdded { name, .. } => name.len(),
            OpKind::DeviceRevoked { .. } => 0,
        }
}

/// True when an operation's contents are within limits.
pub fn op_within_limits(op: &Op) -> bool {
    op.seq >= 1
        && match &op.kind {
            OpKind::ClipboardText(t) => t.len() <= MAX_TEXT_BYTES,
            OpKind::DeviceAdded { name, .. } => name.len() <= MAX_NAME_BYTES,
            OpKind::DeviceRevoked { .. } => true,
        }
}
