//! Console commands on stdin, for scripting, demos and tests. They call the
//! same functions (`api`) as the local control channel, and only format the
//! results as text.

use std::net::SocketAddr;

use anyhow::{anyhow, Result};
use quinn::Endpoint;
use tether_core::DeviceId;
use tether_ipc::DeviceState;

use crate::api;
use crate::ctx::Ctx;

const HELP: &str = "\
commands:
  /pair                  show a pairing code for another device to enter
  /join CODE [ADDR]      pair with a device showing CODE (ADDR = ip:port if it isn't found nearby)
  /peers                 list the devices in this group
  /revoke ID             remove a device (first characters of its id are enough)
  /whoami                show this device's id and group
  /help                  this list";

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

fn state_name(state: DeviceState) -> &'static str {
    match state {
        DeviceState::ThisDevice => "this device",
        DeviceState::Connected => "connected",
        DeviceState::Offline => "offline",
        DeviceState::Revoked => "revoked",
        DeviceState::Nearby => "nearby, not paired",
    }
}

pub async fn handle(ctx: &Ctx, endpoint: &Endpoint, line: &str) {
    let mut words = line.split_whitespace();
    let result: Result<String> = match words.next().unwrap_or("") {
        "/help" => Ok(HELP.to_string()),
        "/whoami" => endpoint.local_addr().map_err(Into::into).map(|addr| {
            let s = api::status(ctx, addr.port());
            format!("this device: {} ({})\ngroup root: {}\nmembers: {}", s.device.to_hex(), s.name, s.root, s.members)
        }),
        "/pair" => api::pair(ctx).map(|p| {
            format!(
                "pairing code: {}\nenter it on the other device with /join {}\nvalid for {} minutes or {} attempts",
                p.code,
                p.code,
                p.valid_secs / 60,
                p.attempts
            )
        }),
        "/join" => match (words.next(), words.next()) {
            (None, _) => Err(anyhow!("usage: /join CODE [ADDR]")),
            (Some(code), addr) => match addr.map(str::parse::<SocketAddr>).transpose() {
                Err(_) => Err(anyhow!("ADDR looks like 192.168.1.20:5000")),
                Ok(addr) => {
                    api::join(ctx, endpoint, code.to_string(), addr).await.map(|d| format!("paired with {d}; syncing"))
                }
            },
        },
        "/peers" => Ok(api::devices(ctx)
            .iter()
            .map(|d| format!("  {}  {:<20} {}", d.id.to_hex(), d.name, state_name(d.state)))
            .collect::<Vec<_>>()
            .join("\n")),
        "/revoke" => match words.next() {
            None => Err(anyhow!("usage: /revoke ID")),
            Some(prefix) => find_member(ctx, prefix).and_then(|d| {
                api::revoke(ctx, d)?;
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
