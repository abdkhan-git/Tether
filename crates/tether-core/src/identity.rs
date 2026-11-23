//! Device identity.
//!
//! A device *is* its Ed25519 key pair. The public key is the [`DeviceId`] that
//! every peer pins after pairing; the secret key stays in the OS keychain and
//! signs every operation the device authors. Nothing else identifies a device,
//! so nothing else can be spoofed.

use std::fmt;
use std::str::FromStr;

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

/// The public half of a device's key pair.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub [u8; 32]);

impl DeviceId {
    /// True when `sig` is a valid signature by this device over `msg`.
    ///
    /// Uses strict verification: rejects non-canonical and small-order
    /// encodings, so a signature cannot be altered and still verify.
    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool {
        match VerifyingKey::from_bytes(&self.0) {
            Ok(key) => key.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(&sig.0)).is_ok(),
            Err(_) => false,
        }
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Display for DeviceId {
    /// Short form: enough to tell devices apart in logs and the UI.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(&self.0[..4]))
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({self})")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDeviceIdError;

impl fmt::Display for ParseDeviceIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a device id is 64 hexadecimal characters")
    }
}

impl std::error::Error for ParseDeviceIdError {}

impl FromStr for DeviceId {
    type Err = ParseDeviceIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut out = [0u8; 32];
        hex::decode_to_slice(s, &mut out).map_err(|_| ParseDeviceIdError)?;
        Ok(Self(out))
    }
}

/// An Ed25519 signature. Serialised as 64 raw bytes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(pub [u8; 64]);

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({}…)", hex::encode(&self.0[..4]))
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = Signature;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("64 signature bytes")
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Signature, E> {
                let bytes: [u8; 64] = v.try_into().map_err(|_| E::invalid_length(v.len(), &self))?;
                Ok(Signature(bytes))
            }
        }
        d.deserialize_bytes(Visitor)
    }
}

/// The secret half of a device's key pair. Zeroed when dropped; never printed.
#[derive(Clone)]
pub struct DeviceKey(SigningKey);

impl DeviceKey {
    /// Rebuild a key from its 32 secret bytes (as stored in the OS keychain).
    pub fn from_secret(secret: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    /// The 32 secret bytes, for storing in the OS keychain. Zeroed on drop.
    pub fn secret(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    pub fn id(&self) -> DeviceId {
        DeviceId(self.0.verifying_key().to_bytes())
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(self.0.sign(msg).to_bytes())
    }

    /// Deterministic keys for tests and the simulator. Never use in production.
    pub fn for_test(n: u64) -> Self {
        let mut secret = [0u8; 32];
        secret[..8].copy_from_slice(&n.to_le_bytes());
        secret[8..16].copy_from_slice(b"testkey!");
        Self::from_secret(&secret)
    }
}

impl fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceKey({})", self.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_verify_only_for_the_signer_and_message() {
        let (a, b) = (DeviceKey::for_test(1), DeviceKey::for_test(2));
        let sig = a.sign(b"hello");
        assert!(a.id().verify(b"hello", &sig));
        assert!(!a.id().verify(b"hellp", &sig));
        assert!(!b.id().verify(b"hello", &sig));
    }

    #[test]
    fn a_tampered_signature_is_rejected() {
        let a = DeviceKey::for_test(1);
        let mut sig = a.sign(b"hello");
        sig.0[10] ^= 1;
        assert!(!a.id().verify(b"hello", &sig));
    }

    #[test]
    fn ids_round_trip_through_hex_and_debug_hides_the_secret() {
        let key = DeviceKey::for_test(7);
        let id = key.id();
        assert_eq!(id.to_hex().parse::<DeviceId>().unwrap(), id);
        assert!("xyz".parse::<DeviceId>().is_err());
        let shown = format!("{key:?}");
        assert!(!shown.contains(&hex::encode(*key.secret())));
    }

    #[test]
    fn signatures_round_trip_through_postcard() {
        let sig = DeviceKey::for_test(3).sign(b"x");
        let bytes = postcard::to_stdvec(&sig).unwrap();
        assert_eq!(postcard::from_bytes::<Signature>(&bytes).unwrap(), sig);
    }
}
