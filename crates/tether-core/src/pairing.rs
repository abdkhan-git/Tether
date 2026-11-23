//! Pairing two devices with a short code.
//!
//! The inviter shows a six-digit code; the joiner types it in. They run SPAKE2,
//! a password-authenticated key exchange, over an encrypted but not yet
//! trusted connection:
//!
//! - An attacker who does not know the code gets **one guess per attempt** and
//!   learns nothing that helps an offline brute force of the code.
//! - Both sides then prove they derived the same key (key confirmation) over a
//!   transcript that includes both device keys and the TLS channel's exported
//!   keying material. A machine-in-the-middle relaying between two TLS
//!   sessions sees different channel bindings, so its confirmations fail.
//!
//! On success each side knows, with the code as proof, which device key is on
//! the other end, and pins it.
//!
//! Flow (joiner J dials inviter I):
//! 1. J → I `PairStart { spake: msg_j }`
//! 2. I → J `PairReply { spake: msg_i, confirm: MAC(k_i) , root, name }`
//! 3. J checks I's MAC, then J → I `PairConfirm { confirm: MAC(k_j), name }`
//! 4. I checks J's MAC, records `DeviceAdded(J)`, replies `PairDone`.

use spake2::{Ed25519Group, Identity, Password, Spake2};

use crate::DeviceId;

/// Digits in a pairing code.
pub const CODE_DIGITS: usize = 6;
/// Failed attempts before a pairing window closes.
pub const MAX_ATTEMPTS: u32 = 3;
/// How long a pairing window stays open.
pub const WINDOW_SECS: u64 = 120;

/// What both sides bind the exchange to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub inviter: DeviceId,
    pub joiner: DeviceId,
    /// TLS exported keying material for this connection (same on both ends).
    pub channel: [u8; 32],
}

/// TLS exporter label both sides use to derive `Binding::channel`.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-tether-pairing-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairError {
    /// The other side's message was not a valid SPAKE2 message.
    BadMessage,
    /// Key confirmation failed: wrong code, or someone in the middle.
    Mismatch,
}

/// Format a random number as a pairing code.
pub fn code_from_random(n: u32) -> String {
    format!("{:06}", n % 1_000_000)
}

/// Is this a well-formed code (exactly six digits)?
pub fn is_valid_code(code: &str) -> bool {
    code.len() == CODE_DIGITS && code.bytes().all(|b| b.is_ascii_digit())
}

fn identities(b: &Binding) -> (Identity, Identity) {
    (Identity::new(&b.inviter.0), Identity::new(&b.joiner.0))
}

fn transcript(b: &Binding, msg_i: &[u8], msg_j: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"tether/pair/v1");
    h.update(&b.inviter.0);
    h.update(&b.joiner.0);
    h.update(&b.channel);
    h.update(&(msg_i.len() as u32).to_le_bytes());
    h.update(msg_i);
    h.update(&(msg_j.len() as u32).to_le_bytes());
    h.update(msg_j);
    *h.finalize().as_bytes()
}

/// The two confirmation MACs for a shared key and transcript.
fn confirmations(shared: &[u8], transcript: &[u8; 32]) -> (blake3::Hash, blake3::Hash) {
    let mut material = Vec::with_capacity(shared.len() + 32);
    material.extend_from_slice(shared);
    material.extend_from_slice(transcript);
    let k_i = blake3::derive_key("tether pairing v1 inviter confirmation", &material);
    let k_j = blake3::derive_key("tether pairing v1 joiner confirmation", &material);
    (blake3::keyed_hash(&k_i, transcript), blake3::keyed_hash(&k_j, transcript))
}

/// Joiner side, between sending `PairStart` and receiving `PairReply`.
pub struct Joiner {
    state: Spake2<Ed25519Group>,
    binding: Binding,
    msg_j: Vec<u8>,
}

impl Joiner {
    /// Start pairing. Returns the state and the message for `PairStart`.
    pub fn start(binding: Binding, code: &str) -> (Self, Vec<u8>) {
        let (id_i, id_j) = identities(&binding);
        let (state, msg_j) = Spake2::<Ed25519Group>::start_b(&Password::new(code), &id_i, &id_j);
        (Self { state, binding, msg_j: msg_j.clone() }, msg_j)
    }

    /// Check the inviter's reply. On success returns the joiner's confirmation
    /// for `PairConfirm`.
    pub fn finish(self, msg_i: &[u8], inviter_confirm: &[u8; 32]) -> Result<[u8; 32], PairError> {
        let shared = self.state.finish(msg_i).map_err(|_| PairError::BadMessage)?;
        let t = transcript(&self.binding, msg_i, &self.msg_j);
        let (expect_i, mine) = confirmations(&shared, &t);
        // blake3::Hash compares in constant time.
        if expect_i != blake3::Hash::from(*inviter_confirm) {
            return Err(PairError::Mismatch);
        }
        Ok(*mine.as_bytes())
    }
}

/// Inviter side, between sending `PairReply` and receiving `PairConfirm`.
pub struct Inviter {
    expect_j: blake3::Hash,
}

impl Inviter {
    /// Respond to `PairStart`. Returns the state, the inviter's SPAKE2
    /// message, and its confirmation for `PairReply`.
    pub fn respond(binding: Binding, code: &str, msg_j: &[u8]) -> Result<(Self, Vec<u8>, [u8; 32]), PairError> {
        let (id_i, id_j) = identities(&binding);
        let (state, msg_i) = Spake2::<Ed25519Group>::start_a(&Password::new(code), &id_i, &id_j);
        let shared = state.finish(msg_j).map_err(|_| PairError::BadMessage)?;
        let t = transcript(&binding, &msg_i, msg_j);
        let (mine, expect_j) = confirmations(&shared, &t);
        Ok((Self { expect_j }, msg_i, *mine.as_bytes()))
    }

    /// Check the joiner's confirmation (constant time).
    pub fn verify(&self, joiner_confirm: &[u8; 32]) -> Result<(), PairError> {
        if self.expect_j == blake3::Hash::from(*joiner_confirm) {
            Ok(())
        } else {
            Err(PairError::Mismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceKey;

    fn binding(channel: u8) -> Binding {
        Binding { inviter: DeviceKey::for_test(1).id(), joiner: DeviceKey::for_test(2).id(), channel: [channel; 32] }
    }

    #[test]
    fn the_same_code_pairs() {
        let (joiner, msg_j) = Joiner::start(binding(7), "482913");
        let (inviter, msg_i, confirm_i) = Inviter::respond(binding(7), "482913", &msg_j).unwrap();
        let confirm_j = joiner.finish(&msg_i, &confirm_i).unwrap();
        assert!(inviter.verify(&confirm_j).is_ok());
    }

    #[test]
    fn a_wrong_code_fails_on_both_sides() {
        let (joiner, msg_j) = Joiner::start(binding(7), "000000");
        let (inviter, msg_i, confirm_i) = Inviter::respond(binding(7), "482913", &msg_j).unwrap();
        assert_eq!(joiner.finish(&msg_i, &confirm_i), Err(PairError::Mismatch));
        assert_eq!(inviter.verify(&[0; 32]), Err(PairError::Mismatch));
    }

    #[test]
    fn a_relay_between_two_channels_is_detected() {
        // Same code, but the two sides see different TLS channels: someone
        // in the middle terminated TLS separately with each of them.
        let (joiner, msg_j) = Joiner::start(binding(1), "482913");
        let (_, msg_i, confirm_i) = Inviter::respond(binding(2), "482913", &msg_j).unwrap();
        assert_eq!(joiner.finish(&msg_i, &confirm_i), Err(PairError::Mismatch));
    }

    #[test]
    fn a_swapped_device_key_is_detected() {
        let mut other = binding(7);
        other.joiner = DeviceKey::for_test(3).id();
        let (joiner, msg_j) = Joiner::start(binding(7), "482913");
        let (_, msg_i, confirm_i) = Inviter::respond(other, "482913", &msg_j).unwrap();
        assert!(joiner.finish(&msg_i, &confirm_i).is_err());
    }

    #[test]
    fn garbage_spake_messages_are_rejected() {
        assert_eq!(Inviter::respond(binding(7), "482913", &[0u8; 5]).err(), Some(PairError::BadMessage));
    }

    #[test]
    fn codes_are_six_digits() {
        assert_eq!(code_from_random(42), "000042");
        assert!(is_valid_code("123456"));
        assert!(!is_valid_code("12345") && !is_valid_code("12345a") && !is_valid_code("１２３４５６"));
    }
}
