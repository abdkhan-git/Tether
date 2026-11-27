//! Core types shared by every Tether component: device identities, clocks,
//! signed operations, the replicated log with membership, the wire protocol,
//! pairing, and the content policy.
//!
//! Nothing in this crate does I/O. That is deliberate: the same code runs in
//! the real daemon and inside the deterministic simulator (`tether-sim`).
//! (Pairing draws its SPAKE2 nonce from the OS random generator.)

#![forbid(unsafe_code)]

pub mod backoff;
pub mod clock;
pub mod content;
pub mod delta;
pub mod hlc;
pub mod identity;
pub mod limits;
pub mod live;
pub mod log;
pub mod notes;
pub mod op;
pub mod pairing;
pub mod pulse;
pub mod shelf;
pub mod wire;

pub use clock::{Clock, SystemClock};
pub use delta::{Delta, Step};
pub use hlc::{Hlc, Timestamp};
pub use identity::{DeviceId, DeviceKey, Signature};
pub use log::{
    derive_trust, ApplyResult, BatchOutcome, ItemId, LocalError, Marks, NoteInfo, NoteRevision, OpLog, Rejection,
    ShelfItem, Trust, VersionVector,
};
pub use op::{item_hash, send_is_fresh, ItemRef, NoteRef, Op, OpKind};
pub use wire::{DecodeError, Message};
