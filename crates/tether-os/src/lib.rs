//! Operating-system integration for Tether.
//!
//! This is the only crate in the workspace allowed to use `unsafe` (SECURITY.md
//! rule 7). Keep it small: each module wraps one OS facility behind a safe API.
//! Every `unsafe` block carries a `// SAFETY:` comment saying why it is sound,
//! and clippy refuses to build one without it.

#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]

pub mod local;
