//! Console commands. Stage P1 moves these behind the desktop app's
//! authenticated local channel; the console stays for scripting and tests.

use std::net::SocketAddr;

use anyhow::{anyhow, Result};
use quinn::Endpoint;
use tether_core::pairing::{code_from_random, MAX_ATTEMPTS, WINDOW_SECS};
use tether_core::DeviceId;

use crate::ctx::Ctx;
use crate::{net, pair};

const HELP: &str = "\
commands:
  /pair                  show a pairing code for another device to enter
  /join CODE [ADDR]      pair with a device showing CODE (ADDR = ip:port if it isn't found nearby)
  /peers                 list the devices in this group
  /revoke ID             remove a device (first characters of its id are enough)
  /whoami                show this device's id and group
  /help                  this list";

/// A uniformly random six-digit code (rejection sampling avoids modulo bias).
fn random_code() -> Result<String> {
    loop {
        let n = getrandom::u32().map_err(|e| anyhow!("no secure randomness available: {e}"))?;
        if n < 4_294_000_000 {
            return Ok(code_from_random(n));
        }
    }
}

fn find_member(ctx: &Ctx, prefix: &str) -> Result<DeviceId> {
    let prefix = prefix.to_ascii_lowercase();
    if prefix.len() < 4 {
        return Err(anyhow!("give at least 4 characters of the device id"));
    }
    let s = ctx.lock();
    let matches: Vec<DeviceId> =
        s.log.trust().members.keys().filter(|d| d.to_hex().starts_with(&prefix)).copied().collect();
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(anyhow!("no member starts with {prefix}")),
        _ => Err(anyhow!("{prefix} matches more than one device; give more characters")),
    }
}

pub async fn handle(ctx: &Ctx, endpoint: &Endpoint, line: &str) {
    let mut words = line.split_whitespace();
    let result: Result<String> = match words.next().unwrap_or("") {
        "/help" => Ok(HELP.to_string()),
        "/whoami" => {
            let s = ctx.lock();
            Ok(format!("this device: {} ({})\ngroup root: {}\nmembers: {}", s.log.me().to_hex(), s.name, s.log.root(), s.log.trust().members.len()))
        }
        "/pair" => random_code().map(|code| {
            ctx.open_pairing(code.clone());
            format!("pairing code: {code}\nenter it on the other device with /join {code}\nvalid for {} minutes or {MAX_ATTEMPTS} attempts", WINDOW_SECS / 60)
        }),
        "/join" => match words.next() {
            None => Err(anyhow!("usage: /join CODE [ADDR]")),
            Some(code) => {
                let candidates: Vec<SocketAddr> = match words.next() {
                    Some(addr) => addr.parse().map(|a| vec![a]).map_err(|_| anyhow!("ADDR looks like 192.168.1.20:5000")),
                    None => {
                        let s = ctx.lock();
                        let trusted = ctx.gate.read().map(|g| g.trusted.clone()).unwrap_or_default();
                        Ok(s.nearby.iter().filter(|(d, _)| !trusted.contains(d)).map(|(_, a)| *a).collect())
                    }
                }
                .unwrap_or_default();
                pair::join(ctx.clone(), endpoint.clone(), code.to_string(), candidates).await.map(|d| format!("paired with {d}; syncing"))
            }
        },
        "/peers" => {
            let s = ctx.lock();
            let me = s.log.me();
            let trust = s.log.trust();
            let lines: Vec<String> = trust
                .members
                .iter()
                .map(|(d, added_as)| {
                    let name = s.names.get(d).cloned().unwrap_or_else(|| added_as.clone());
                    let state = if *d == me {
                        "this device"
                    } else if trust.cutoffs.contains_key(d) {
                        "revoked"
                    } else if s.peers.contains_key(d) {
                        "connected"
                    } else {
                        "offline"
                    };
                    format!("  {}  {:<20} {state}", d.to_hex(), name)
                })
                .collect();
            Ok(lines.join("\n"))
        }
        "/revoke" => match words.next() {
            None => Err(anyhow!("usage: /revoke ID")),
            Some(prefix) => find_member(ctx, prefix).and_then(|d| {
                let op = ctx.revoke(d)?;
                net::broadcast(ctx, op);
                Ok(format!("revoked {d}; its future operations will be rejected by every device"))
            }),
        },
        "" => return,
        other => Err(anyhow!("unknown command {other}; try /help")),
    };
    match result {
        Ok(text) => println!("{text}"),
        Err(e) => println!("error: {e}"),
    }
}
