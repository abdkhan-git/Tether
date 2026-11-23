//! Secrets kept in the OS credential store (Windows Credential Manager, macOS
//! Keychain, Secret Service on Linux) and never written to a file or a log:
//! the device's signing key and the key that encrypts its database.

use anyhow::{anyhow, Context, Result};
use tether_core::DeviceKey;
use zeroize::{Zeroize, Zeroizing};

use crate::store::DbKey;

const SERVICE: &str = "Tether";

/// 32 bytes from the operating system's random number generator.
fn random32() -> Result<Zeroizing<[u8; 32]>> {
    let mut secret = Zeroizing::new([0u8; 32]);
    getrandom::fill(secret.as_mut()).map_err(|e| anyhow!("no secure randomness available: {e}"))?;
    Ok(secret)
}

/// Load a 32-byte secret from the credential store, creating it on first run.
fn load_or_create_secret(account: &str, what: &str) -> Result<Zeroizing<[u8; 32]>> {
    let entry = keyring::Entry::new(SERVICE, account).context("opening the OS credential store")?;
    match entry.get_secret() {
        Ok(mut bytes) => {
            let parsed: Result<[u8; 32], _> = bytes.as_slice().try_into();
            bytes.zeroize();
            Ok(Zeroizing::new(parsed.map_err(|_| anyhow!("the stored {what} is corrupt"))?))
        }
        Err(keyring::Error::NoEntry) => {
            let secret = random32()?;
            entry
                .set_secret(secret.as_ref())
                .with_context(|| format!("saving the {what} to the OS credential store"))?;
            Ok(secret)
        }
        Err(e) => Err(e).with_context(|| format!("reading the {what} from the OS credential store")),
    }
}

/// A fresh device key (for `--ephemeral` runs).
pub fn generate() -> Result<DeviceKey> {
    Ok(DeviceKey::from_secret(&*random32()?))
}

/// This profile's device key, created on first run.
pub fn load_or_create(profile: &str) -> Result<DeviceKey> {
    let secret = load_or_create_secret(&format!("device-key/{profile}"), "device key")?;
    Ok(DeviceKey::from_secret(&secret))
}

/// The key that encrypts this profile's database, created on first run.
pub fn load_or_create_db_key(profile: &str) -> Result<DbKey> {
    Ok(DbKey::new(load_or_create_secret(&format!("db-key/{profile}"), "database key")?))
}

/// Is `name` safe to use as a profile (in a credential name and a folder name)?
pub fn valid_profile(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
