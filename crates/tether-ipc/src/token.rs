//! Where the control token lives: the OS credential store, next to the
//! device key, under `control-token/<profile>`. The daemon replaces it at
//! every start, so a token copied earlier stops working on restart.
//!
//! `TETHER_CONTROL_TOKEN` (64 hex digits) supplies the token instead. It
//! exists for `--ephemeral` daemons in tests and demos, which must not touch
//! the credential store; a normal daemon ignores it.

use anyhow::{anyhow, Context, Result};
use zeroize::{Zeroize, Zeroizing};

use crate::auth::Token;

pub const TOKEN_ENV: &str = "TETHER_CONTROL_TOKEN";
const SERVICE: &str = "Tether";

fn entry(profile: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, &format!("control-token/{profile}")).context("opening the OS credential store")
}

/// The token from `TETHER_CONTROL_TOKEN`, if that is set.
pub fn from_env() -> Option<Result<Token>> {
    let mut text = std::env::var(TOKEN_ENV).ok()?;
    let token = Token::from_hex(&text).ok_or_else(|| anyhow!("{TOKEN_ENV} must be 64 hex digits"));
    text.zeroize();
    Some(token)
}

/// Daemon: make a fresh token for this run and store it for local clients.
pub fn publish(profile: &str) -> Result<Token> {
    let token = Token::generate()?;
    entry(profile)?.set_secret(token.as_bytes()).context("saving the control token to the OS credential store")?;
    Ok(token)
}

/// Daemon, on a clean shutdown: remove the token.
pub fn withdraw(profile: &str) -> Result<()> {
    match entry(profile)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("removing the control token"),
    }
}

/// Client: the running daemon's token (`TETHER_CONTROL_TOKEN` first).
pub fn load(profile: &str) -> Result<Token> {
    if let Some(token) = from_env() {
        return token;
    }
    match entry(profile)?.get_secret() {
        Ok(mut bytes) => {
            let parsed: Result<[u8; 32], _> = bytes.as_slice().try_into();
            bytes.zeroize();
            let bytes = parsed.map_err(|_| anyhow!("the stored control token is corrupt; restart tetherd"))?;
            Ok(Token::from_bytes(Zeroizing::new(bytes)))
        }
        Err(keyring::Error::NoEntry) => Err(anyhow!("no control token for profile {profile}: is tetherd running?")),
        Err(e) => Err(e).context("reading the control token from the OS credential store"),
    }
}
