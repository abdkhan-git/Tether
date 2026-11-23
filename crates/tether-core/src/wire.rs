//! Messages exchanged between peers. One message per QUIC bi-directional
//! stream, answered by one reply on the same stream.

use serde::{Deserialize, Serialize};

use crate::{DeviceId, Op, VersionVector};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    /// Sent when a connection opens. The reply is `Ops` with everything the
    /// sender is missing, so a device that was offline catches up at once.
    Hello {
        device: DeviceId,
        name: String,
        vv: VersionVector,
    },
    /// New operations. The reply is `Ack`.
    Ops(Vec<Op>),
    Ack {
        applied: u32,
    },
}

impl Message {
    /// Upper bound on one encoded message. Large files will travel as chunks
    /// on their own streams, never inside a `Message`.
    pub const MAX_FRAME: usize = 16 * 1024 * 1024;

    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("Message is always serializable")
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpKind, OpLog};

    #[test]
    fn round_trips() {
        let mut log = OpLog::new(DeviceId::from_u128(9));
        let op = log.append_local(OpKind::ClipboardText("héllo 👋".into()), 42);
        for msg in [
            Message::Hello { device: log.me(), name: "laptop".into(), vv: log.version_vector() },
            Message::Ops(vec![op]),
            Message::Ack { applied: 1 },
        ] {
            assert_eq!(Message::decode(&msg.encode()).unwrap(), msg);
        }
    }
}
