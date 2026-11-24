use serde::{Deserialize, Serialize};

use crate::{DeviceId, DeviceKey, Signature, Timestamp};

/// One immutable change, appended to its author's log and signed by its
/// author's device key.
///
/// `(device, seq)` names every operation uniquely, like a Kafka offset with
/// one partition per device. Applying the same operation twice is a no-op.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub device: DeviceId,
    pub seq: u64,
    pub ts: Timestamp,
    pub kind: OpKind,
    /// Signature by `device` over every field above (see [`Op::signed_bytes`]).
    pub sig: Signature,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    /// Text copied on `device`.
    ClipboardText(String),
    /// The author vouches for `device` joining the group (recorded at pairing).
    DeviceAdded { device: DeviceId, name: String },
    /// `device` is removed from the group. Its operations with a higher `seq`
    /// than `after_seq` are rejected by every device; earlier ones remain,
    /// because history is immutable.
    DeviceRevoked { device: DeviceId, after_seq: u64 },
    /// Pin or unpin a clipboard item. The latest pin for an item wins
    /// (by [`Op::order_key`], then `seq`), the same on every device.
    Pin { item: ItemRef, pinned: bool },
    /// Remove a clipboard item from history on every device. Final: no pin,
    /// earlier or later, brings it back.
    Delete { item: ItemRef },
    /// Give a member a display name on every device. The latest rename wins
    /// (by [`Op::order_key`], then `seq`). Names are display only, never trusted.
    Rename { device: DeviceId, name: String },
    /// Put a history item on one device's clipboard. Only `to` acts on it,
    /// and only while it is fresh (see [`send_is_fresh`]); it changes no
    /// shared state, so every other device just keeps it in the log.
    Send { item: ItemRef, to: DeviceId },
}

/// How old a `Send` may be when it arrives and still be applied.
pub const SEND_FRESH_MS: u64 = 120_000;
/// How far in the future a `Send` may claim to be (clock skew allowance).
pub const SEND_FUTURE_MS: u64 = 30_000;

/// Should a `Send` stamped `sent_ms` be applied at local time `now_ms`? A send
/// that arrives late (the target was offline) is not applied: the item is in
/// history, and an old request should not overwrite the clipboard hours later.
pub fn send_is_fresh(sent_ms: u64, now_ms: u64) -> bool {
    sent_ms <= now_ms.saturating_add(SEND_FUTURE_MS) && now_ms.saturating_sub(sent_ms) <= SEND_FRESH_MS
}

/// A clipboard item, as a pin or delete names it: the operation that copied
/// it, and a hash of its text. A pin or delete takes effect only on an item
/// whose text matches the hash, so a device cannot aim one at a copy it has
/// not seen (such as another device's future copies).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ItemRef {
    pub device: DeviceId,
    pub seq: u64,
    pub hash: [u8; 32],
}

/// Domain-separated BLAKE3 hash of an item's text, for [`ItemRef::hash`].
pub fn item_hash(text: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("tether/item/v1");
    h.update(text.as_bytes());
    *h.finalize().as_bytes()
}

/// Domain separation: a signature over an operation can never be confused
/// with a signature over anything else this key signs (such as a TLS handshake).
const OP_DOMAIN: &str = "tether/op/v1";

impl Op {
    /// Create and sign an operation.
    pub fn sign(key: &DeviceKey, seq: u64, ts: Timestamp, kind: OpKind) -> Self {
        let device = key.id();
        let sig = key.sign(&Self::signed_bytes(device, seq, ts, &kind));
        Self { device, seq, ts, kind, sig }
    }

    /// The exact bytes covered by the signature.
    pub fn signed_bytes(device: DeviceId, seq: u64, ts: Timestamp, kind: &OpKind) -> Vec<u8> {
        postcard::to_stdvec(&(OP_DOMAIN, device, seq, ts, kind)).expect("operations always serialise")
    }

    /// True when the signature is valid for the claimed author.
    pub fn signature_is_valid(&self) -> bool {
        self.device.verify(&Self::signed_bytes(self.device, self.seq, self.ts, &self.kind), &self.sig)
    }

    pub fn id(&self) -> (DeviceId, u64) {
        (self.device, self.seq)
    }

    /// Total order used for clipboard history and for resolving membership:
    /// HLC time, ties broken by device id. Every device computes the same
    /// order from the same set of operations.
    pub fn order_key(&self) -> (Timestamp, DeviceId) {
        (self.ts, self.device)
    }

    pub fn is_membership(&self) -> bool {
        matches!(self.kind, OpKind::DeviceAdded { .. } | OpKind::DeviceRevoked { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_change_to_a_signed_operation_breaks_the_signature() {
        let key = DeviceKey::for_test(1);
        let op = Op::sign(&key, 1, Timestamp { wall_ms: 5, counter: 0 }, OpKind::ClipboardText("hi".into()));
        assert!(op.signature_is_valid());

        let mut changed = op.clone();
        changed.kind = OpKind::ClipboardText("rm -rf ~".into());
        assert!(!changed.signature_is_valid());

        let mut moved = op.clone();
        moved.seq = 2;
        assert!(!moved.signature_is_valid());

        let mut impostor = op;
        impostor.device = DeviceKey::for_test(2).id();
        assert!(!impostor.signature_is_valid());
    }
}
