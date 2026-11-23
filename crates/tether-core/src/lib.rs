//! Core types shared by every Tether component: device identities, clocks,
//! operations, the replicated operation log and the wire protocol.
//!
//! Nothing in this crate does I/O. That is deliberate: the same code runs in
//! the real daemon and inside the deterministic simulator (`tether-sim`).

pub mod clock;
pub mod hlc;
pub mod id;
pub mod log;
pub mod op;
pub mod wire;

pub use clock::{Clock, SystemClock};
pub use hlc::{Hlc, Timestamp};
pub use id::DeviceId;
pub use log::{ApplyResult, OpLog, VersionVector};
pub use op::{Op, OpKind};
pub use wire::Message;
