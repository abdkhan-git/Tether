//! Requests, replies and events on the control channel.
//!
//! One connection carries either requests (each answered in order) or, after
//! `Subscribe`, a stream of events. A client wanting both opens two.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use tether_core::content::HoldReason;
use tether_core::{DeviceId, Timestamp};

/// Bumped on any change to the messages below; both ends must match.
pub const PROTOCOL_VERSION: u16 = 1;
/// Largest request a client may send.
pub const MAX_REQUEST_BYTES: usize = 16 << 10;
/// Largest reply or event the daemon sends (one clipboard item can be 1 MiB).
pub const MAX_REPLY_BYTES: usize = 4 << 20;
/// Most items in one history page.
pub const MAX_HISTORY_PAGE: u16 = 100;
/// Byte budget of one history page; a page always holds at least one item.
pub const HISTORY_PAGE_BYTES: usize = 2 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Status,
    Devices,
    /// Clipboard history, newest first, starting after `before`.
    History {
        before: Option<Cursor>,
        limit: u16,
    },
    /// Open a pairing window and return its code.
    Pair,
    /// Join the device showing `code`, found nearby or at `addr`.
    Join {
        code: String,
        addr: Option<SocketAddr>,
    },
    Revoke {
        device: DeviceId,
    },
    /// Turn this connection into a stream of [`Event`]s.
    Subscribe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Ok(Reply),
    Err(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Status(Status),
    Devices(Vec<Device>),
    History(HistoryPage),
    PairCode(PairCode),
    Joined { device: DeviceId },
    Revoked,
    Subscribed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub device: DeviceId,
    pub name: String,
    pub root: DeviceId,
    pub members: u32,
    pub connected: u32,
    /// This device was removed from its group and does not sync.
    pub revoked: bool,
    /// UDP port the daemon listens on for other devices.
    pub port: u16,
    pub pairing_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    /// Display only: chosen by the device itself, never trusted.
    pub name: String,
    pub state: DeviceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceState {
    ThisDevice,
    Connected,
    Offline,
    Revoked,
    /// Seen on the network, not paired.
    Nearby,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCode {
    pub code: String,
    pub valid_secs: u32,
    pub attempts: u32,
}

/// A position in history: items strictly older than this come next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub at: Timestamp,
    pub device: DeviceId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub author: DeviceId,
    pub seq: u64,
    pub at: Timestamp,
    pub text: String,
    /// Set when the item came from another device and was kept out of the
    /// system clipboard (it looks like a command or hides characters).
    pub held: Option<HoldReason>,
}

impl HistoryItem {
    pub fn cursor(&self) -> Cursor {
        Cursor { at: self.at, device: self.author }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub items: Vec<HistoryItem>,
    /// Pass as `before` for the next page; `None` at the end.
    pub next: Option<Cursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// A new history item, copied here or received. Usually the newest, but
    /// one that arrives late belongs further down: place it by its cursor.
    Clipboard(HistoryItem),
    /// Many items arrived at once (catching up); fetch history again.
    HistoryChanged,
    /// Something copied here looked like a secret; it stays on this device.
    KeptLocal { kind: String },
    /// Membership, connections, nearby devices or the pairing window changed.
    DevicesChanged,
    /// This subscriber fell behind and missed events; refresh everything.
    Lagged { missed: u64 },
}
