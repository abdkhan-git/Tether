//! The device's secret key, kept in the OS credential store (Windows
//! Credential Manager, macOS Keychain, Secret Service on Linux) and never
//! written to a file or a log.

use anyhow::{anyhow, Context, Result};
use tether_core::DeviceKey;
use zeroize::{Zeroize, Zeroizing};

const SERVICE: &str = "Tether";

fn account(profile: &str) -> String {
    format!("device-key/{profile}")
}

/// A fresh key from the operating system's random number generator.
pub fn generate() -> Result<DeviceKey> {
    let mut secret = Zeroizing::new([0u8; 32]);
    getrandom::fill(secret.as_mut()).map_err(|e| anyhow!("no secure randomness available: {e}"))?;
    Ok(DeviceKey::from_secret(&secret))
}

/// Load this profile's key from the credential store, creating it on first run.
pub fn load_or_create(profile: &str) -> Result<DeviceKey> {
    let entry = keyring::Entry::new(SERVICE, &account(profile)).context("opening the OS credential store")?;
    match entry.get_secret() {
        Ok(mut bytes) => {
            let parsed: Result<[u8; 32], _> = bytes.as_slice().try_into();
            bytes.zeroize();
            let mut secret = parsed.map_err(|_| anyhow!("the stored device key is corrupt"))?;
            let key = DeviceKey::from_secret(&secret);
            secret.zeroize();
            Ok(key)
        }
        Err(keyring::Error::NoEntry) => {
            let key = generate()?;
            entry.set_secret(key.secret().as_ref()).context("saving the device key to the OS credential store")?;
            Ok(key)
        }
        Err(e) => Err(e).context("reading the device key from the OS credential store"),
    }
}

/// Is `name` safe to use as a profile (in a credential name and a folder name)?
pub fn valid_profile(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
