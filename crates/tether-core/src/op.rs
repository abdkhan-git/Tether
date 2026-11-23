use serde::{Deserialize, Serialize};

use crate::{DeviceId, Timestamp};

/// One immutable change, appended to its author's log.
///
/// `(device, seq)` names every operation uniquely, like a Kafka offset with
/// one partition per device. Applying the same operation twice is a no-op.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub device: DeviceId,
    pub seq: u64,
    pub ts: Timestamp,
    pub kind: OpKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    /// Text copied on `device`. Later stages add files, pins, deletes and workspaces.
    ClipboardText(String),
}

impl Op {
    pub fn id(&self) -> (DeviceId, u64) {
        (self.device, self.seq)
    }

    /// Total order used for clipboard history: HLC time, ties broken by device id.
    /// Every device computes the same order from the same set of operations.
    pub fn order_key(&self) -> (Timestamp, DeviceId) {
        (self.ts, self.device)
    }
}
