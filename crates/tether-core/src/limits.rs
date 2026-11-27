//! Hard limits on everything a peer can send. Every decoded message is checked
//! against these before any of it is used (SECURITY.md, rule 6).

use crate::notes::{MAX_NOTEBOOK_BYTES, MAX_TITLE_BYTES};
use crate::shelf::{safe_name, text_root, ShelfKind, MAX_INLINE_BYTES, MAX_ITEM_BYTES, MAX_TTL_MS};
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
            OpKind::DeviceAdded { name, .. } | OpKind::Rename { name, .. } => name.len(),
            OpKind::DeviceRevoked { .. } | OpKind::Pin { .. } | OpKind::Delete { .. } | OpKind::Send { .. } => 0,
            OpKind::NoteCreate { title } => title.len(),
            OpKind::NoteEdit { edit, .. } => {
                edit.inserts.iter().map(|r| r.text.len() + 48).sum::<usize>() + edit.deletes.len() * 48
            }
            OpKind::NoteMeta { title, notebook, .. } => {
                title.as_ref().map_or(0, String::len) + notebook.as_ref().map_or(0, String::len)
            }
            OpKind::ShelfAdd { name, text, .. } => name.len() + text.as_ref().map_or(0, String::len),
            OpKind::ShelfRemove { .. } | OpKind::ShelfPin { .. } => 0,
        }
}

/// A shelf item's description is consistent and within limits: a name that is
/// already safe, a size within bounds, and for text exactly the inline text
/// with its root (files and folders carry no text).
fn shelf_add_ok(kind: ShelfKind, name: &str, size: u64, root: &[u8; 32], text: &Option<String>, ttl_ms: u64) -> bool {
    let inline = matches!(kind, ShelfKind::Text | ShelfKind::Link);
    let text_ok = match (inline, text) {
        (true, Some(t)) => t.len() <= MAX_INLINE_BYTES && size == t.len() as u64 && *root == text_root(t),
        (false, None) => size <= MAX_ITEM_BYTES,
        _ => false,
    };
    text_ok && name.len() <= crate::shelf::MAX_NAME_BYTES && safe_name(name) == name && ttl_ms <= MAX_TTL_MS
}

/// True when an operation's contents are within limits.
pub fn op_within_limits(op: &Op) -> bool {
    op.seq >= 1
        && match &op.kind {
            OpKind::ClipboardText(t) => t.len() <= MAX_TEXT_BYTES,
            OpKind::DeviceAdded { name, .. } | OpKind::Rename { name, .. } => name.len() <= MAX_NAME_BYTES,
            OpKind::DeviceRevoked { .. } | OpKind::Pin { .. } | OpKind::Delete { .. } | OpKind::Send { .. } => true,
            OpKind::NoteCreate { title } => title.len() <= MAX_TITLE_BYTES,
            OpKind::NoteEdit { edit, .. } => edit.within_limits() && edit.ids_belong_to(op.seq),
            OpKind::NoteMeta { title, notebook, .. } => {
                title.as_ref().map_or(true, |t| t.len() <= MAX_TITLE_BYTES)
                    && notebook.as_ref().map_or(true, |n| n.len() <= MAX_NOTEBOOK_BYTES)
            }
            OpKind::ShelfAdd { kind, name, size, root, text, ttl_ms } => {
                shelf_add_ok(*kind, name, *size, root, text, *ttl_ms)
            }
            OpKind::ShelfRemove { .. } | OpKind::ShelfPin { .. } => true,
        }
}
