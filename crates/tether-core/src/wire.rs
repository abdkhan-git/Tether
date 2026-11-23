//! Messages exchanged between peers. One message per QUIC bi-directional
//! stream, answered by one reply on the same stream.
//!
//! A peer's identity is never taken from a message: it comes from the key in
//! its TLS certificate, verified during the handshake.

use serde::{Deserialize, Serialize};

use crate::limits::{
    op_within_limits, MAX_FRAME, MAX_NAME_BYTES, MAX_OPS_PER_MESSAGE, MAX_VV_ENTRIES, SPAKE_MSG_BYTES,
};
use crate::{DeviceId, Op, VersionVector};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// Sent when a connection opens. The reply is `Ops` with what the sender
    /// is missing (one page; `more` asks it to pull again).
    Hello {
        name: String,
        vv: VersionVector,
    },
    /// Operations. The reply to a pushed `Ops` is `Ack`.
    Ops {
        ops: Vec<Op>,
        more: bool,
    },
    Ack {
        applied: u32,
    },

    /// Pairing, joiner → inviter: the joiner's SPAKE2 message.
    PairStart {
        spake: Vec<u8>,
    },
    /// Pairing, inviter → joiner: the inviter's SPAKE2 message, its key
    /// confirmation, and the group the joiner is joining.
    PairReply {
        spake: Vec<u8>,
        confirm: [u8; 32],
        root: DeviceId,
        name: String,
    },
    /// Pairing, joiner → inviter: the joiner's key confirmation.
    PairConfirm {
        confirm: [u8; 32],
        name: String,
    },
    /// Pairing finished; both sides now trust each other.
    PairDone,
    /// Pairing refused. Deliberately carries no reason.
    PairFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooLarge,
    Malformed,
    TrailingBytes,
    OutOfLimits,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DecodeError::TooLarge => "message larger than the frame limit",
            DecodeError::Malformed => "message could not be decoded",
            DecodeError::TrailingBytes => "message has trailing bytes",
            DecodeError::OutOfLimits => "message exceeds protocol limits",
        })
    }
}

impl std::error::Error for DecodeError {}

impl Message {
    /// Upper bound on one encoded message. Large files travel as chunks on
    /// their own streams, never inside a `Message`.
    pub const MAX_FRAME: usize = MAX_FRAME;

    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("Message is always serializable")
    }

    /// Decode and check against every protocol limit. Anything that fails is
    /// dropped by the caller without being used.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() > MAX_FRAME {
            return Err(DecodeError::TooLarge);
        }
        let (msg, rest) = postcard::take_from_bytes::<Message>(bytes).map_err(|_| DecodeError::Malformed)?;
        if !rest.is_empty() {
            return Err(DecodeError::TrailingBytes);
        }
        if !msg.within_limits() {
            return Err(DecodeError::OutOfLimits);
        }
        Ok(msg)
    }

    fn within_limits(&self) -> bool {
        match self {
            Message::Hello { name, vv } => name.len() <= MAX_NAME_BYTES && vv.len() <= MAX_VV_ENTRIES,
            Message::Ops { ops, .. } => ops.len() <= MAX_OPS_PER_MESSAGE && ops.iter().all(op_within_limits),
            Message::Ack { .. } | Message::PairDone | Message::PairFailed => true,
            Message::PairStart { spake } => spake.len() == SPAKE_MSG_BYTES,
            Message::PairReply { spake, name, .. } => spake.len() == SPAKE_MSG_BYTES && name.len() <= MAX_NAME_BYTES,
            Message::PairConfirm { name, .. } => name.len() <= MAX_NAME_BYTES,
        }
    }

    /// Pairing messages are the only ones an unpaired peer may send.
    pub fn is_pairing(&self) -> bool {
        matches!(self, Message::PairStart { .. } | Message::PairConfirm { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceKey, OpKind, OpLog};

    #[test]
    fn round_trips() {
        let mut log = OpLog::new(DeviceKey::for_test(9));
        let op = log.append_local(OpKind::ClipboardText("héllo 👋".into()), 42).unwrap();
        for msg in [
            Message::Hello { name: "laptop".into(), vv: log.version_vector() },
            Message::Ops { ops: vec![op], more: false },
            Message::Ack { applied: 1 },
            Message::PairStart { spake: vec![1; SPAKE_MSG_BYTES] },
            Message::PairReply { spake: vec![2; SPAKE_MSG_BYTES], confirm: [3; 32], root: log.me(), name: "pc".into() },
            Message::PairConfirm { confirm: [4; 32], name: "mac".into() },
            Message::PairDone,
            Message::PairFailed,
        ] {
            assert_eq!(Message::decode(&msg.encode()).unwrap(), msg);
        }
    }

    #[test]
    fn oversized_names_and_trailing_bytes_are_refused() {
        let long = Message::Hello { name: "x".repeat(MAX_NAME_BYTES + 1), vv: VersionVector::new() };
        assert_eq!(Message::decode(&long.encode()), Err(DecodeError::OutOfLimits));
        let mut bytes = Message::PairDone.encode();
        bytes.push(0);
        assert_eq!(Message::decode(&bytes), Err(DecodeError::TrailingBytes));
        assert_eq!(Message::decode(&vec![0u8; MAX_FRAME + 1]), Err(DecodeError::TooLarge));
    }

    #[test]
    fn a_huge_length_prefix_does_not_allocate_or_panic() {
        // Variant 0 (Hello) followed by a name length of u64::MAX as a varint.
        let bytes = [0u8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        assert!(Message::decode(&bytes).is_err());
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(4_000))]

        /// No input, however hostile, makes decoding panic.
        #[test]
        fn decode_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = Message::decode(&bytes);
        }

        /// Mutating a valid message never panics, and whatever decodes is within limits.
        #[test]
        fn mutated_messages_are_safe(flip in 0usize..400, bit in 0u8..8) {
            let mut log = crate::OpLog::new(crate::DeviceKey::for_test(1));
            let op = log.append_local(crate::OpKind::ClipboardText("some text".into()), 1).unwrap();
            let mut bytes = Message::Ops { ops: vec![op], more: false }.encode();
            let i = flip % bytes.len();
            bytes[i] ^= 1 << bit;
            if let Ok(Message::Ops { ops, .. }) = Message::decode(&bytes) {
                prop_assert!(ops.len() <= MAX_OPS_PER_MESSAGE);
            }
        }
    }
}
