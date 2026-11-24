//! The `tether` command. Every subcommand is a client of the daemon's local
//! control channel (`tether_ipc`); nothing here reads the database or keys.
//!
//! Commands write to any `Write` and read confirmations from any `BufRead`,
//! so tests drive them in-process against real daemons.

#![forbid(unsafe_code)]

pub mod term;

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};
use tether_core::content::HoldReason;
use tether_core::DeviceId;
use tether_ipc::{
    token, Client, Device, DeviceState, Endpoint, Event, HistoryItem, PairCode, Reply, Request, Status, Token,
    MAX_HISTORY_PAGE,
};

use crate::term::{ago, preview, safe, short};

#[derive(Parser, Debug)]
#[command(name = "tether", version, about = "Control the Tether daemon on this computer")]
pub struct Cli {
    /// The daemon to talk to: the --profile tetherd was started with.
    #[arg(long, global = true, default_value = "default")]
    pub profile: String,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Command {
    /// Show this device, its group and its connections.
    Status,
    /// List the devices in the group, and unpaired devices nearby.
    Devices,
    /// Show clipboard history, newest first.
    History {
        /// How many items to show.
        #[arg(long, short = 'n', default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=10_000))]
        limit: u32,
        /// Print each item in full instead of a one-line preview.
        #[arg(long)]
        full: bool,
    },
    /// Print copies and device changes as they happen.
    Watch {
        /// Stop after this many new clipboard items.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        count: Option<u32>,
    },
    /// Show a pairing code for another device to enter, then wait for it to join.
    Pair {
        /// Print the code and return without waiting.
        #[arg(long)]
        no_wait: bool,
    },
    /// Join the device showing CODE: found nearby, or at ADDR (for example 192.168.1.20:5000).
    Join { code: String, addr: Option<SocketAddr> },
    /// Remove a device from the group. ID may be the first characters of its id.
    Revoke {
        id: String,
        /// Do not ask for confirmation.
        #[arg(long, short)]
        yes: bool,
    },
    /// Stop syncing: copies here stay here, and other devices' copies only reach history.
    Pause,
    /// Start syncing again after `tether pause`.
    Resume,
}

/// Which daemon to talk to, and the token that proves we may.
pub struct Target {
    pub endpoint: Endpoint,
    pub token: Token,
}

impl Target {
    pub fn for_profile(profile: &str) -> Result<Self> {
        Ok(Self { endpoint: Endpoint::for_profile(profile)?, token: token::load(profile)? })
    }

    async fn connect(&self) -> Result<Client> {
        Client::connect_to(&self.endpoint, &self.token).await
    }
}

pub async fn run<W: Write, R: BufRead>(command: Command, target: &Target, out: &mut W, input: &mut R) -> Result<()> {
    match command {
        Command::Status => status_cmd(target, out).await,
        Command::Devices => devices_cmd(target, out).await,
        Command::History { limit, full } => history_cmd(target, out, limit, full).await,
        Command::Watch { count } => watch_cmd(target, out, count).await,
        Command::Pair { no_wait } => pair_cmd(target, out, no_wait).await,
        Command::Join { code, addr } => join_cmd(target, out, code, addr).await,
        Command::Revoke { id, yes } => revoke_cmd(target, out, input, &id, yes).await,
        Command::Pause => pause_cmd(target, out, true).await,
        Command::Resume => pause_cmd(target, out, false).await,
    }
}

// --- Typed requests ---

fn unexpected(reply: Reply) -> anyhow::Error {
    anyhow!("unexpected reply from tetherd: {reply:?}")
}

async fn status(c: &mut Client) -> Result<Status> {
    match c.request(&Request::Status).await? {
        Reply::Status(s) => Ok(s),
        r => Err(unexpected(r)),
    }
}

async fn devices(c: &mut Client) -> Result<Vec<Device>> {
    match c.request(&Request::Devices).await? {
        Reply::Devices(d) => Ok(d),
        r => Err(unexpected(r)),
    }
}

// --- Formatting ---

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn state_text(state: DeviceState) -> &'static str {
    match state {
        DeviceState::ThisDevice => "this device",
        DeviceState::Connected => "connected",
        DeviceState::Offline => "offline",
        DeviceState::Revoked => "revoked",
        DeviceState::Nearby => "nearby, not paired",
    }
}

fn held_text(reason: HoldReason) -> &'static str {
    match reason {
        HoldReason::HiddenCharacters => "hides control characters",
        HoldReason::DangerousCommand => "looks like a dangerous command",
        HoldReason::MultiLineCommand => "multi-line shell command",
    }
}

fn is_member(state: DeviceState) -> bool {
    !matches!(state, DeviceState::Nearby)
}

/// Display names by device, with this device called "this device".
fn names(devices: &[Device]) -> HashMap<DeviceId, String> {
    devices
        .iter()
        .map(|d| {
            let name = match d.state {
                DeviceState::ThisDevice => "this device".to_string(),
                _ if d.name.is_empty() => short(&d.id),
                _ => preview(&d.name, 20),
            };
            (d.id, name)
        })
        .collect()
}

fn who(names: &HashMap<DeviceId, String>, id: &DeviceId) -> String {
    names.get(id).cloned().unwrap_or_else(|| short(id))
}

fn held_note(item: &HistoryItem) -> String {
    item.held.map(|r| format!("  [held: {}]", held_text(r))).unwrap_or_default()
}

// --- Commands ---

async fn status_cmd<W: Write>(target: &Target, out: &mut W) -> Result<()> {
    let mut c = target.connect().await?;
    let s = status(&mut c).await?;
    writeln!(out, "This device  {}  {}", preview(&s.name, 40), s.device.to_hex())?;
    writeln!(out, "Group        {} member(s), {} connected (root {})", s.members, s.connected, short(&s.root))?;
    writeln!(out, "Network      UDP port {}", s.port)?;
    writeln!(out, "Pairing      {}", if s.pairing_open { "open" } else { "closed" })?;
    writeln!(out, "Sync         {}", if s.paused { "paused (run `tether resume`)" } else { "on" })?;
    if s.revoked {
        writeln!(out, "This device was removed from its group and does not sync. Pair it again to rejoin.")?;
    }
    Ok(())
}

async fn devices_cmd<W: Write>(target: &Target, out: &mut W) -> Result<()> {
    let mut c = target.connect().await?;
    writeln!(out, "{:<14}{:<24}STATE", "ID", "NAME")?;
    for d in devices(&mut c).await? {
        writeln!(out, "{:<14}{:<24}{}", short(&d.id), preview(&d.name, 22), state_text(d.state))?;
    }
    Ok(())
}

async fn history_cmd<W: Write>(target: &Target, out: &mut W, limit: u32, full: bool) -> Result<()> {
    let mut c = target.connect().await?;
    let names = names(&devices(&mut c).await?);
    let now = now_ms();
    let (mut shown, mut before) = (0u32, None);
    while shown < limit {
        let want = (limit - shown).min(u32::from(MAX_HISTORY_PAGE)) as u16;
        let page = match c.request(&Request::History { before, limit: want }).await? {
            Reply::History(p) => p,
            r => return Err(unexpected(r)),
        };
        for item in &page.items {
            let (when, from) = (ago(item.at.wall_ms, now), who(&names, &item.author));
            if full {
                writeln!(out, "{when} · {from}{}", held_note(item))?;
                for line in safe(&item.text, true).lines() {
                    writeln!(out, "    {line}")?;
                }
                writeln!(out)?;
            } else {
                writeln!(out, "{when:>10}  {from:<20}  {}{}", preview(&item.text, 60), held_note(item))?;
            }
        }
        shown += page.items.len() as u32;
        match page.next {
            Some(next) if !page.items.is_empty() => before = Some(next),
            _ => break,
        }
    }
    if shown == 0 {
        writeln!(out, "No clipboard history yet.")?;
    }
    Ok(())
}

async fn watch_cmd<W: Write>(target: &Target, out: &mut W, count: Option<u32>) -> Result<()> {
    // Subscribe first, so nothing between the name lookup and the stream is missed.
    let mut events = target.connect().await?.subscribe().await?;
    let mut c = target.connect().await?;
    let mut names = names(&devices(&mut c).await?);
    let mut items = 0u32;
    if count.is_none() {
        writeln!(out, "Watching (Ctrl+C to stop)...")?;
    }
    loop {
        match events.next().await? {
            Event::Clipboard(item) => {
                if !names.contains_key(&item.author) {
                    names = self::names(&devices(&mut c).await?);
                }
                let from = who(&names, &item.author);
                writeln!(out, "copied on {from}: {}{}", preview(&item.text, 70), held_note(&item))?;
                items += 1;
                if count.is_some_and(|n| items >= n) {
                    return Ok(());
                }
            }
            Event::HistoryChanged => writeln!(out, "many items arrived; run `tether history` to see them")?,
            Event::KeptLocal { kind } => {
                writeln!(out, "kept a copied {} on this device; it was not synced", safe(&kind, false))?
            }
            Event::DevicesChanged => names = self::names(&devices(&mut c).await?),
            Event::Paused(paused) => writeln!(out, "sync {}", if paused { "paused" } else { "resumed" })?,
            Event::ItemChanged { author, deleted: true, .. } => {
                writeln!(out, "an item from {} was deleted", who(&names, &author))?
            }
            Event::ItemChanged { .. } => {}
            Event::Lagged { missed } => writeln!(out, "missed {missed} events; run `tether history` to catch up")?,
        }
        out.flush()?;
    }
}

async fn pair_cmd<W: Write>(target: &Target, out: &mut W, no_wait: bool) -> Result<()> {
    let mut c = target.connect().await?;
    let members: HashSet<DeviceId> =
        devices(&mut c).await?.into_iter().filter(|d| is_member(d.state)).map(|d| d.id).collect();
    // Subscribe before opening the window, so the join cannot slip past.
    let mut events = if no_wait { None } else { Some(target.connect().await?.subscribe().await?) };
    let PairCode { code, valid_secs, attempts } = match c.request(&Request::Pair).await? {
        Reply::PairCode(p) => p,
        r => return Err(unexpected(r)),
    };
    let code = safe(&code, false);
    writeln!(out, "Pairing code: {code}")?;
    writeln!(out, "On the other device, run: tether join {code}")?;
    writeln!(out, "The code works for {} minutes or {attempts} attempts.", valid_secs / 60)?;
    let Some(events) = events.as_mut() else { return Ok(()) };
    writeln!(out, "Waiting for the other device (Ctrl+C to stop)...")?;
    out.flush()?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(u64::from(valid_secs) + 2);
    loop {
        let event = tokio::time::timeout_at(deadline, events.next())
            .await
            .map_err(|_| anyhow!("the code expired before a device joined"))??;
        if !matches!(event, Event::DevicesChanged | Event::Lagged { .. }) {
            continue;
        }
        let now = devices(&mut c).await?;
        if let Some(d) = now.iter().find(|d| is_member(d.state) && !members.contains(&d.id)) {
            writeln!(out, "Paired with {} ({}).", preview(&d.name, 40), short(&d.id))?;
            return Ok(());
        }
        if !status(&mut c).await?.pairing_open {
            bail!("the pairing window closed before a device joined (too many wrong codes?)");
        }
    }
}

async fn pause_cmd<W: Write>(target: &Target, out: &mut W, paused: bool) -> Result<()> {
    let mut c = target.connect().await?;
    match c.request(&Request::SetPaused { paused }).await? {
        Reply::Paused(true) => writeln!(out, "Sync paused. Copies stay on this device until you run `tether resume`.")?,
        Reply::Paused(false) => writeln!(out, "Sync resumed.")?,
        r => return Err(unexpected(r)),
    }
    Ok(())
}

async fn join_cmd<W: Write>(target: &Target, out: &mut W, code: String, addr: Option<SocketAddr>) -> Result<()> {
    let mut c = target.connect().await?;
    match c.request(&Request::Join { code, addr }).await? {
        Reply::Joined { device } => {
            writeln!(out, "Paired with {}; syncing.", short(&device))?;
            Ok(())
        }
        r => Err(unexpected(r)),
    }
}

async fn revoke_cmd<W: Write, R: BufRead>(
    target: &Target,
    out: &mut W,
    input: &mut R,
    id: &str,
    yes: bool,
) -> Result<()> {
    let prefix = id.trim().to_ascii_lowercase();
    if prefix.len() < 4 {
        bail!("give at least 4 characters of the device id (see `tether devices`)");
    }
    let mut c = target.connect().await?;
    let list = devices(&mut c).await?;
    let matches: Vec<&Device> =
        list.iter().filter(|d| is_member(d.state) && d.id.to_hex().starts_with(&prefix)).collect();
    let device = match matches.as_slice() {
        [] => bail!("no device in the group starts with {}", safe(&prefix, false)),
        [d] if d.state == DeviceState::ThisDevice => {
            bail!("that is this device; revoke it from another device in the group")
        }
        [d] if d.state == DeviceState::Revoked => bail!("{} is already revoked", short(&d.id)),
        [d] => *d,
        _ => bail!("{} matches more than one device; give more characters", safe(&prefix, false)),
    };
    let name = preview(&device.name, 40);
    if !yes {
        write!(
            out,
            "Revoke {name} ({})? It will be cut off from every device in the group. [y/N] ",
            short(&device.id)
        )?;
        out.flush()?;
        let mut answer = String::new();
        input.read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            writeln!(out, "Not revoked.")?;
            return Ok(());
        }
    }
    match c.request(&Request::Revoke { device: device.id }).await? {
        Reply::Revoked => {
            writeln!(out, "Revoked {name} ({}).", short(&device.id))?;
            Ok(())
        }
        r => Err(unexpected(r)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("tether").chain(args.iter().copied()))
    }

    #[test]
    fn arguments_parse_as_documented() {
        assert_eq!(
            parse(&["history", "-n", "5", "--full"]).unwrap().command,
            Command::History { limit: 5, full: true }
        );
        assert_eq!(parse(&["history"]).unwrap().command, Command::History { limit: 20, full: false });
        assert_eq!(parse(&["pause"]).unwrap().command, Command::Pause);
        assert_eq!(parse(&["resume"]).unwrap().command, Command::Resume);
        assert_eq!(parse(&["revoke", "abcd", "-y"]).unwrap().command, Command::Revoke { id: "abcd".into(), yes: true });
        assert_eq!(
            parse(&["join", "123456", "192.168.1.20:5000"]).unwrap().command,
            Command::Join { code: "123456".into(), addr: Some("192.168.1.20:5000".parse().unwrap()) }
        );
        let cli = parse(&["status", "--profile", "left"]).unwrap();
        assert_eq!((cli.profile.as_str(), cli.command), ("left", Command::Status));
    }

    #[test]
    fn bad_arguments_are_refused() {
        assert!(parse(&["history", "-n", "0"]).is_err());
        assert!(parse(&["join", "123456", "not-an-address"]).is_err());
        assert!(parse(&["watch", "--count", "0"]).is_err());
        assert!(parse(&["revoke"]).is_err());
        assert!(parse(&[]).is_err());
    }
}
