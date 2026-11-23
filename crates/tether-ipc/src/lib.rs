//! The local control channel between `tetherd` and its clients: the desktop
//! app and the `tether` CLI. Threat model: docs/SECURITY.md, "Local control
//! channel".
//!
//! Layers, bottom up:
//! - `tether_os::local`: a pipe or socket only the current user can open.
//! - [`frame`]: length-prefixed postcard frames with hard size limits.
//! - [`auth`]: both ends prove they know the daemon's token, which never
//!   crosses the channel itself.
//! - [`proto`]: the requests, replies and events.
//! - [`client`]: what the app and CLI use.

#![forbid(unsafe_code)]

pub mod auth;
pub mod client;
pub mod frame;
pub mod proto;
pub mod token;

pub use auth::Token;
pub use client::{authenticated_stream, client_stream, Client, Events};
pub use proto::*;
pub use tether_os::local::{Endpoint, Listener, Stream};
