//! Pulse (F10): heartbeats between connected devices, each device's health,
//! failure detection, and waking a device that is asleep. Threat model:
//! docs/SECURITY.md, "Pulse".
//!
//! Beats travel as QUIC datagrams on the existing, mutually authenticated
//! connection: small, unreliable, never stored and never in the log. A peer
//! that sends more than a few a second has the rest dropped.

use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};
use quinn::Connection;
use serde::{Deserialize, Serialize};
use tether_core::pulse::{self, Battery, Beat, Detector, Health, BEAT_MS, MAX_MACS, WATCH_MS};
use tether_core::DeviceId;
use tether_ipc::{DevicePulse, Event};
use tether_os::health::{Sample, Sampler};
use tracing::{debug, warn};

use crate::ctx::{Ctx, State};

/// Beats a peer may send per second before the rest are dropped.
const MAX_BEATS_PER_SECOND: u32 = 4;
/// A device is told about low storage at most this often.
const LOW_STORAGE_EVERY: Duration = Duration::from_secs(24 * 3600);
/// Wake packets to one device at most this often.
const WAKE_EVERY: Duration = Duration::from_secs(5);
/// "Last heard" is saved at most this often while a device stays connected.
const SAVE_HEARD_EVERY: Duration = Duration::from_secs(600);
/// Wake-on-LAN listens on the discard port (9); some adapters use 7.
const WAKE_PORTS: [u16; 2] = [9, 7];

/// What is remembered across restarts, per device: how to wake it, and when
/// it was last heard from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Known {
    macs: Vec<[u8; 6]>,
    heard_unix_ms: u64,
}

pub struct PulseState {
    epoch: Instant,
    /// This device's latest health.
    local: Option<Health>,
    /// Someone here is looking at devices, until then.
    watched_until: Instant,
    peers: HashMap<DeviceId, Peer>,
    book: BTreeMap<DeviceId, Known>,
    book_saved: HashMap<DeviceId, Instant>,
    warned: HashMap<DeviceId, Instant>,
    woken: HashMap<DeviceId, Instant>,
}

struct Peer {
    health: Option<Health>,
    detector: Detector,
    watching_until: Instant,
    /// Start of the current one-second window, and beats in it.
    window: (Instant, u32),
}

fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl PulseState {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            epoch: now,
            local: None,
            watched_until: now,
            peers: HashMap::new(),
            book: BTreeMap::new(),
            book_saved: HashMap::new(),
            warned: HashMap::new(),
            woken: HashMap::new(),
        }
    }

    fn ms(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.epoch).as_millis() as u64
    }

    /// Someone here or on a connected peer is looking.
    fn watched(&self, now: Instant) -> bool {
        now < self.watched_until || self.peers.values().any(|p| now < p.watching_until)
    }

    /// Load what was saved; unreadable means start empty.
    pub fn load_book(&mut self, bytes: &[u8]) {
        if let Ok(book) = postcard::from_bytes::<BTreeMap<DeviceId, Known>>(bytes) {
            self.book = book;
        }
    }

    fn book_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(&self.book).unwrap_or_default()
    }
}

/// This device's health from a reading.
fn health_of(s: Sample) -> Health {
    Health {
        cpu_permille: s.cpu_permille.min(1000),
        mem_used: s.mem_used,
        mem_total: s.mem_total,
        disk_free: s.disk_free,
        disk_total: s.disk_total,
        battery: s.battery.map(|(percent, charging)| Battery { percent, charging }),
        uptime_s: s.uptime_s,
        os: crate::ctx::clean_text(&s.os, 48),
        version: env!("CARGO_PKG_VERSION").to_string(),
        macs: s.macs.into_iter().filter(|m| pulse::wakeable(*m)).take(MAX_MACS).collect(),
    }
}

impl Ctx {
    /// The beat to send now, once this device has sampled its health.
    pub fn beat(&self) -> Option<Beat> {
        let s = self.lock();
        let watching = Instant::now() < s.pulse.watched_until;
        s.pulse.local.clone().map(|health| Beat { watching, health })
    }

    /// Someone here is looking at devices.
    pub fn watch(&self) {
        self.lock().pulse.watched_until = Instant::now() + Duration::from_millis(WATCH_MS);
    }

    /// A datagram from `peer` (already authenticated by its connection).
    pub fn heard(&self, peer: DeviceId, bytes: &[u8]) {
        let now = Instant::now();
        let mut low = None;
        {
            let mut s = self.lock();
            if !s.log.trust().is_active(peer) {
                return;
            }
            let at = s.pulse.ms(now);
            let p = s.pulse.peers.entry(peer).or_insert_with(|| Peer {
                health: None,
                detector: Detector::new(),
                watching_until: now,
                window: (now, 0),
            });
            if now.duration_since(p.window.0) >= Duration::from_secs(1) {
                p.window = (now, 0);
            }
            p.window.1 += 1;
            if p.window.1 > MAX_BEATS_PER_SECOND {
                return;
            }
            let beat = match Beat::decode(bytes) {
                Ok(beat) => beat,
                Err(e) => {
                    debug!(%peer, ?e, "dropped a malformed beat");
                    return;
                }
            };
            p.detector.beat(at);
            if beat.watching {
                p.watching_until = now + Duration::from_millis(WATCH_MS);
            }
            if beat.health.low_storage() {
                low = Some((beat.health.disk_free, beat.health.disk_total));
            }
            let macs: Vec<[u8; 6]> = beat.health.macs.iter().copied().filter(|m| pulse::wakeable(*m)).collect();
            p.health = Some(beat.health);
            // Remember how to wake it, and roughly when it was last here.
            let known = s.pulse.book.entry(peer).or_default();
            let changed = known.macs != macs;
            known.macs = macs;
            known.heard_unix_ms = unix_ms();
            let stale = s.pulse.book_saved.get(&peer).map_or(true, |t| now.duration_since(*t) >= SAVE_HEARD_EVERY);
            if changed || stale {
                s.pulse.book_saved.insert(peer, now);
                save_book(&mut s);
            }
        }
        if let Some((free, total)) = low {
            self.low_storage(peer, free, total);
        }
    }

    /// A connection to `peer` ended: note when it was last heard from.
    pub fn pulse_gone(&self, peer: DeviceId) {
        let mut s = self.lock();
        if s.pulse.peers.remove(&peer).is_some() {
            if let Some(known) = s.pulse.book.get_mut(&peer) {
                known.heard_unix_ms = unix_ms();
                save_book(&mut s);
            }
        }
    }

    /// This device's latest health.
    pub fn sampled(&self, sample: Sample) {
        let health = health_of(sample);
        let low = health.low_storage().then_some((health.disk_free, health.disk_total));
        let me = {
            let mut s = self.lock();
            s.pulse.local = Some(health);
            s.log.me()
        };
        if let Some((free, total)) = low {
            self.low_storage(me, free, total);
        }
    }

    fn low_storage(&self, device: DeviceId, free: u64, total: u64) {
        let now = Instant::now();
        {
            let mut s = self.lock();
            if s.pulse.warned.get(&device).is_some_and(|t| now.duration_since(*t) < LOW_STORAGE_EVERY) {
                return;
            }
            s.pulse.warned.insert(device, now);
        }
        self.emit(Event::LowStorage { device, free, total });
    }

    /// How sampling should go: often while anyone is looking.
    fn sample_every(&self) -> Duration {
        let s = self.lock();
        let now = Instant::now();
        let watched_until = if s.pulse.watched(now) { u64::MAX } else { 0 };
        Duration::from_millis(pulse::sample_every(watched_until, 0))
    }

    /// What Pulse knows about `device`, for the Devices page.
    pub(crate) fn device_pulse(s: &State, device: DeviceId, connected: bool) -> Option<DevicePulse> {
        let now = Instant::now();
        if device == s.log.me() {
            return s.pulse.local.clone().map(|h| DevicePulse {
                health: Some(h),
                heard_ms_ago: Some(0),
                liveness: None,
                wakeable: false,
            });
        }
        let peer = s.pulse.peers.get(&device);
        let known = s.pulse.book.get(&device);
        if peer.is_none() && known.is_none() {
            return None;
        }
        let heard_ms_ago = match peer.and_then(|p| p.detector.last_beat()) {
            Some(last) => Some(s.pulse.ms(now).saturating_sub(last)),
            None => known.filter(|k| k.heard_unix_ms > 0).map(|k| unix_ms().saturating_sub(k.heard_unix_ms)),
        };
        Some(DevicePulse {
            health: peer.and_then(|p| p.health.clone()),
            heard_ms_ago,
            liveness: peer.filter(|_| connected).and_then(|p| p.detector.liveness(s.pulse.ms(now))),
            wakeable: known.is_some_and(|k| !k.macs.is_empty()),
        })
    }

    /// Send Wake-on-LAN packets to `device`'s adapters. Only to addresses the
    /// device itself reported over its authenticated connection, only for a
    /// current member, and at most once every few seconds.
    pub fn wake(&self, device: DeviceId) -> Result<u32> {
        let macs = self.wake_targets(device)?;
        send_wake(&macs)
    }

    /// The rules of [`Ctx::wake`]: the addresses to wake, if waking is allowed now.
    fn wake_targets(&self, device: DeviceId) -> Result<Vec<[u8; 6]>> {
        let macs = {
            let mut s = self.lock();
            if device == s.log.me() {
                bail!("that is this device");
            }
            if !s.log.trust().is_active(device) {
                bail!("that device is not an active member");
            }
            let macs = s.pulse.book.get(&device).map(|k| k.macs.clone()).unwrap_or_default();
            if macs.is_empty() {
                bail!("that device has not said how to wake it yet: it needs to be online with Tether once");
            }
            let now = Instant::now();
            if s.pulse.woken.get(&device).is_some_and(|t| now.duration_since(*t) < WAKE_EVERY) {
                bail!("a wake-up was just sent; give it a few seconds");
            }
            s.pulse.woken.insert(device, now);
            macs
        };
        Ok(macs)
    }
}

/// Broadcast a magic packet for each address on the local network.
fn send_wake(macs: &[[u8; 6]]) -> Result<u32> {
    {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| anyhow!("cannot send: {e}"))?;
        socket.set_broadcast(true).map_err(|e| anyhow!("cannot send: {e}"))?;
        let mut sent = 0u32;
        for mac in macs {
            let packet = pulse::magic_packet(*mac);
            for port in WAKE_PORTS {
                match socket.send_to(&packet, SocketAddr::from((Ipv4Addr::BROADCAST, port))) {
                    Ok(_) => sent += u32::from(port == WAKE_PORTS[0]),
                    Err(e) => warn!(error = %e, "could not send a wake packet"),
                }
            }
        }
        if sent == 0 {
            bail!("no wake packet could be sent on this network");
        }
        Ok(sent)
    }
}

fn save_book(s: &mut State) {
    let members: Vec<DeviceId> = s.log.trust().members.keys().copied().collect();
    s.pulse.book.retain(|d, _| members.contains(d));
    let bytes = s.pulse.book_bytes();
    Ctx::save_wake_book(s, &bytes);
}

/// Sample this device's health for as long as the daemon runs: every 2 s
/// while anyone is looking at devices, every 30 s otherwise.
pub async fn sample_loop(ctx: Ctx) {
    let sampler = Arc::new(Mutex::new(Sampler::new()));
    let mut last: Option<Instant> = None;
    loop {
        let due = last.map_or(true, |t| t.elapsed() >= ctx.sample_every());
        if due {
            let s = sampler.clone();
            let reading =
                tokio::task::spawn_blocking(move || s.lock().unwrap_or_else(|p| p.into_inner()).sample()).await;
            match reading {
                Ok(sample) => ctx.sampled(sample),
                Err(e) => warn!(error = %e, "could not read this device's health"),
            }
            last = Some(Instant::now());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Beat to `conn` every [`BEAT_MS`] until it closes.
pub async fn beat_loop(ctx: Ctx, conn: Connection) {
    let mut tick = tokio::time::interval(Duration::from_millis(BEAT_MS));
    loop {
        tick.tick().await;
        if conn.close_reason().is_some() {
            return;
        }
        if let Some(beat) = ctx.beat() {
            if let Err(e) = conn.send_datagram(beat.encode().into()) {
                debug!(error = %e, "could not send a beat");
            }
        }
    }
}

/// Take in `peer`'s beats until the connection closes.
pub async fn listen_loop(ctx: Ctx, conn: Connection, peer: DeviceId) {
    while let Ok(bytes) = conn.read_datagram().await {
        ctx.heard(peer, &bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::tests::{ctx, peer_of};

    fn beat(macs: Vec<[u8; 6]>, free: u64) -> Vec<u8> {
        let health = Health {
            mem_total: 1,
            disk_free: free,
            disk_total: 1 << 40,
            os: "Windows 11".into(),
            macs,
            ..Default::default()
        };
        Beat { watching: true, health }.encode()
    }

    const MAC: [u8; 6] = [0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E];

    #[test]
    fn beats_from_members_are_heard_and_others_are_not() {
        let (ctx, _) = ctx();
        let peer = peer_of(&ctx, 2).me();
        let stranger = tether_core::DeviceKey::for_test(9).id();
        ctx.heard(stranger, &beat(vec![MAC], 1 << 39));
        ctx.heard(peer, &beat(vec![MAC, [0x02, 0, 0, 0, 0, 1]], 1 << 39));
        let s = ctx.lock();
        assert!(!s.pulse.peers.contains_key(&stranger));
        let p = Ctx::device_pulse(&s, peer, true).unwrap();
        assert_eq!(p.health.unwrap().os, "Windows 11");
        assert!(p.wakeable);
        assert_eq!(s.pulse.book[&peer].macs, vec![MAC], "only a real hardware address is kept");
        assert!(s.pulse.watched(Instant::now()), "the peer said someone is looking");
    }

    #[test]
    fn a_flood_of_beats_and_junk_is_dropped() {
        let (ctx, _) = ctx();
        let peer = peer_of(&ctx, 2).me();
        for _ in 0..50 {
            ctx.heard(peer, &beat(vec![], 1 << 39));
        }
        ctx.heard(peer, &[0xFF; 3000]);
        let s = ctx.lock();
        let p = &s.pulse.peers[&peer];
        assert_eq!(p.window.1, 51, "every arrival is counted");
        assert!(p.detector.last_beat().is_some());
    }

    #[test]
    fn low_storage_is_told_once_a_day() {
        let (ctx, _) = ctx();
        let peer = peer_of(&ctx, 2).me();
        let mut events = ctx.events.subscribe();
        ctx.heard(peer, &beat(vec![], 1 << 30));
        std::thread::sleep(Duration::from_millis(1100));
        ctx.heard(peer, &beat(vec![], 1 << 30));
        assert!(matches!(events.try_recv(), Ok(Event::LowStorage { device, .. }) if device == peer));
        assert!(events.try_recv().is_err(), "not twice");
    }

    #[test]
    fn waking_is_only_for_members_with_an_address_and_not_too_often() {
        let (ctx, _) = ctx();
        let peer = peer_of(&ctx, 2).me();
        // The rules only: no packet is put on the network by a test.
        assert!(ctx.wake_targets(ctx.me()).is_err());
        assert!(ctx.wake_targets(tether_core::DeviceKey::for_test(9).id()).is_err());
        assert!(ctx.wake_targets(peer).unwrap_err().to_string().contains("not said how to wake"));
        ctx.heard(peer, &beat(vec![MAC], 1 << 39));
        assert_eq!(ctx.wake_targets(peer).unwrap(), vec![MAC]);
        assert!(ctx.wake_targets(peer).unwrap_err().to_string().contains("just sent"));
    }

    #[test]
    fn how_to_wake_a_device_survives_a_restart() {
        let (ctx, _) = ctx();
        let peer = peer_of(&ctx, 2).me();
        ctx.heard(peer, &beat(vec![MAC], 1 << 39));
        let bytes = ctx.lock().pulse.book_bytes();
        let mut fresh = PulseState::new();
        fresh.load_book(&bytes);
        assert_eq!(fresh.book[&peer].macs, vec![MAC]);
        fresh.load_book(b"\xFFjunk");
        assert_eq!(fresh.book.len(), 1, "junk leaves it as it was");
    }

    #[test]
    fn this_device_reports_its_own_health() {
        let (ctx, _) = ctx();
        assert!(ctx.beat().is_none(), "nothing to say before the first reading");
        ctx.sampled(Sample {
            cpu_permille: 2000,
            mem_total: 8,
            mem_used: 4,
            os: "Win\u{1b}dows".into(),
            ..Default::default()
        });
        let beat = ctx.beat().unwrap();
        assert_eq!(beat.health.cpu_permille, 1000, "clamped");
        assert_eq!(beat.health.os, "Windows");
        assert!(beat.health.within_limits());
        ctx.watch();
        assert!(ctx.beat().unwrap().watching);
        assert_eq!(ctx.sample_every(), Duration::from_millis(pulse::SAMPLE_FAST_MS));
    }
}
