//! Pulse (F10): how each device is doing, and whether it is still there.
//!
//! Every connected device sends a small [`Beat`] every [`BEAT_MS`] as a QUIC
//! datagram: unreliable, never stored, never in the log. It carries the
//! sender's health. Two things are read from the beats:
//!
//! - **Health**: the latest [`Health`] a device reported, shown on its card.
//! - **Liveness**: a phi-accrual failure detector ([`Detector`], Hayashibara et
//!   al. 2004, as Cassandra and Akka use it). Instead of a fixed timeout it asks
//!   how unlikely the current silence is, given how regularly beats have been
//!   arriving. A device that goes to sleep is suspected within a few seconds and
//!   called down within about eight, while a busy network that delays a beat or
//!   two raises no alarm.
//!
//! Pure, like the rest of this crate: time is passed in.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// A beat every this many milliseconds.
pub const BEAT_MS: u64 = 2_000;
/// How long a peer's "someone is watching" lasts without another beat saying so.
pub const WATCH_MS: u64 = 10_000;
/// Health is sampled this often while someone watches, and this often otherwise.
pub const SAMPLE_FAST_MS: u64 = 2_000;
pub const SAMPLE_SLOW_MS: u64 = 30_000;
/// Largest encoded beat (it must fit one datagram on any path).
pub const MAX_BEAT_BYTES: usize = 1024;
/// Most network adapters a device reports for waking it.
pub const MAX_MACS: usize = 4;
const MAX_OS_BYTES: usize = 48;
const MAX_VERSION_BYTES: usize = 16;

/// How a device is doing, as it reports itself. Display only: a member can
/// claim anything here, and nothing is decided from it except where to send a
/// wake-up packet (see SECURITY.md, "Pulse").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    /// CPU in use across all cores, in tenths of a percent (0 to 1000).
    pub cpu_permille: u16,
    pub mem_used: u64,
    pub mem_total: u64,
    /// The disk that holds the user's files.
    pub disk_free: u64,
    pub disk_total: u64,
    pub battery: Option<Battery>,
    pub uptime_s: u64,
    /// "Windows 11", "macOS 15.2", "Ubuntu 24.04".
    pub os: String,
    /// Tether's version on that device.
    pub version: String,
    /// Hardware addresses of its wired and wireless adapters, for Wake-on-LAN.
    pub macs: Vec<[u8; 6]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
}

impl Health {
    pub fn within_limits(&self) -> bool {
        self.cpu_permille <= 1000
            && self.mem_used <= self.mem_total
            && self.disk_free <= self.disk_total
            && self.battery.map_or(true, |b| b.percent <= 100)
            && self.os.len() <= MAX_OS_BYTES
            && self.version.len() <= MAX_VERSION_BYTES
            && self.macs.len() <= MAX_MACS
    }

    /// Little room left on the disk: under 5 % or under 5 GB.
    pub fn low_storage(&self) -> bool {
        self.disk_total > 0 && (self.disk_free < 5_000_000_000 || self.disk_free * 20 < self.disk_total)
    }
}

/// One heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beat {
    /// Someone on the sender is looking at its devices: send health often.
    pub watching: bool,
    pub health: Health,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeatError {
    TooLarge,
    Malformed,
    OutOfLimits,
}

impl Beat {
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("beats always serialise")
    }

    /// Strict: bounded, exact, and within limits.
    pub fn decode(bytes: &[u8]) -> Result<Self, BeatError> {
        if bytes.len() > MAX_BEAT_BYTES {
            return Err(BeatError::TooLarge);
        }
        let beat: Beat = match postcard::take_from_bytes(bytes) {
            Ok((beat, [])) => beat,
            _ => return Err(BeatError::Malformed),
        };
        if !beat.health.within_limits() {
            return Err(BeatError::OutOfLimits);
        }
        Ok(beat)
    }
}

/// What the failure detector makes of a device's silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Liveness {
    /// Beats arrive as expected.
    Live,
    /// Quieter than usual: it may be busy, or on its way out.
    Suspect,
    /// Silent for long enough that it is almost certainly gone (asleep,
    /// switched off, out of range).
    Down,
}

/// Phi-accrual failure detector over beat arrival times.
#[derive(Debug, Clone)]
pub struct Detector {
    /// Recent gaps between beats, in milliseconds.
    gaps: VecDeque<u64>,
    last: Option<u64>,
}

/// Gaps remembered.
const WINDOW: usize = 100;
/// A floor on the spread of gaps: on a quiet LAN beats are so regular that
/// a few milliseconds late would otherwise look alarming.
const MIN_STD_MS: f64 = 500.0;
/// Silence allowed on top of the usual gap before suspicion grows (a busy
/// machine, a Wi-Fi hiccup, a garbage-collection pause).
const PAUSE_MS: f64 = 3_000.0;
/// Phi at which a device is suspected, and at which it is called down. A phi
/// of 8 means the silence would happen by chance about once in 10^8 beats.
pub const PHI_SUSPECT: f64 = 3.0;
pub const PHI_DOWN: f64 = 8.0;

impl Default for Detector {
    fn default() -> Self {
        Self::new()
    }
}

impl Detector {
    pub fn new() -> Self {
        Self { gaps: VecDeque::with_capacity(WINDOW), last: None }
    }

    /// A beat arrived at `now_ms`.
    pub fn beat(&mut self, now_ms: u64) {
        if let Some(last) = self.last {
            if now_ms <= last {
                return; // a duplicate, or a clock that did not move
            }
            if self.gaps.len() == WINDOW {
                self.gaps.pop_front();
            }
            self.gaps.push_back(now_ms - last);
        }
        self.last = Some(now_ms);
    }

    pub fn last_beat(&self) -> Option<u64> {
        self.last
    }

    /// How suspicious the silence since the last beat is, at `now_ms`.
    /// Until a few gaps are known, the expected gap is [`BEAT_MS`].
    pub fn phi(&self, now_ms: u64) -> f64 {
        let Some(last) = self.last else { return 0.0 };
        let n = self.gaps.len() as f64;
        let (mean, std) = if self.gaps.len() < 3 {
            (BEAT_MS as f64, MIN_STD_MS)
        } else {
            let mean = self.gaps.iter().sum::<u64>() as f64 / n;
            let var = self.gaps.iter().map(|&g| (g as f64 - mean).powi(2)).sum::<f64>() / n;
            (mean, var.sqrt().max(MIN_STD_MS))
        };
        let elapsed = now_ms.saturating_sub(last) as f64;
        phi(elapsed, mean + PAUSE_MS, std)
    }

    pub fn liveness(&self, now_ms: u64) -> Option<Liveness> {
        self.last?;
        let p = self.phi(now_ms);
        Some(if p >= PHI_DOWN {
            Liveness::Down
        } else if p >= PHI_SUSPECT {
            Liveness::Suspect
        } else {
            Liveness::Live
        })
    }
}

/// -log10 of the chance that a gap is at least `elapsed`, for gaps normally
/// distributed around `mean`. The logistic approximation of the normal tail
/// that Akka uses: accurate to about 0.1 % and never NaN.
fn phi(elapsed: f64, mean: f64, std: f64) -> f64 {
    let y = (elapsed - mean) / std;
    let e = (-y * (1.5976 + 0.070566 * y * y)).exp();
    let p = if elapsed > mean { e / (1.0 + e) } else { 1.0 - 1.0 / (1.0 + e) };
    // Far into the tail `p` underflows to 0; phi is then simply very large.
    -p.max(1e-300).log10()
}

/// Whether health should be sampled often: someone here, or on a peer that
/// said so in a beat within [`WATCH_MS`], is looking.
pub fn sample_every(watched_until_ms: u64, now_ms: u64) -> u64 {
    if now_ms < watched_until_ms {
        SAMPLE_FAST_MS
    } else {
        SAMPLE_SLOW_MS
    }
}

/// The Wake-on-LAN "magic packet" for an adapter: six 0xFF bytes, then its
/// address sixteen times.
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut out = [0xFF; 102];
    for k in 0..16 {
        out[6 + 6 * k..12 + 6 * k].copy_from_slice(&mac);
    }
    out
}

/// A hardware address worth waking: not all zeros, not broadcast, not multicast
/// (lowest bit of the first byte), and not a locally administered one, which
/// virtual adapters and randomised Wi-Fi addresses use.
pub fn wakeable(mac: [u8; 6]) -> bool {
    mac != [0; 6] && mac != [0xFF; 6] && mac[0] & 0b11 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn steady(every: u64, n: u64) -> Detector {
        let mut d = Detector::new();
        for k in 0..n {
            d.beat(k * every);
        }
        d
    }

    #[test]
    fn a_device_that_beats_regularly_is_live_and_one_that_stops_is_down_within_seconds() {
        let d = steady(BEAT_MS, 30);
        let last = 29 * BEAT_MS;
        assert_eq!(d.liveness(last + 1_000), Some(Liveness::Live));
        assert_eq!(d.liveness(last + 4_000), Some(Liveness::Live), "one late beat is no alarm");
        let suspected = (last..).step_by(100).find(|&t| d.liveness(t) != Some(Liveness::Live)).unwrap() - last;
        let down = (last..).step_by(100).find(|&t| d.liveness(t) == Some(Liveness::Down)).unwrap() - last;
        assert!((5_000..=8_000).contains(&suspected), "suspected after {suspected} ms");
        assert!((6_000..=10_000).contains(&down), "down after {down} ms");
        assert!(Detector::new().liveness(5).is_none(), "nothing heard, nothing known");
    }

    #[test]
    fn a_jittery_link_earns_more_patience() {
        let mut jittery = Detector::new();
        let mut t = 0;
        for k in 0..60 {
            t += if k % 2 == 0 { 500 } else { 3_500 };
            jittery.beat(t);
        }
        let steady = steady(BEAT_MS, 60);
        let after = |d: &Detector| d.phi(d.last_beat().unwrap() + 7_000);
        assert!(after(&jittery) < after(&steady), "{} vs {}", after(&jittery), after(&steady));
    }

    #[test]
    fn beats_and_packets_are_checked() {
        let beat = Beat {
            watching: true,
            health: Health {
                cpu_permille: 125,
                mem_used: 8 << 30,
                mem_total: 16 << 30,
                disk_free: 100 << 30,
                disk_total: 500 << 30,
                battery: Some(Battery { percent: 80, charging: false }),
                uptime_s: 3600,
                os: "Windows 11".into(),
                version: "0.1.0".into(),
                macs: vec![[0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E]],
            },
        };
        let bytes = beat.encode();
        assert!(bytes.len() < 200, "a beat is small: {}", bytes.len());
        assert_eq!(Beat::decode(&bytes).unwrap(), beat);
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(Beat::decode(&trailing), Err(BeatError::Malformed));
        assert_eq!(Beat::decode(&vec![0; MAX_BEAT_BYTES + 1]), Err(BeatError::TooLarge));
        for bad in [
            Health { cpu_permille: 1001, ..beat.health.clone() },
            Health { mem_used: 2, mem_total: 1, ..beat.health.clone() },
            Health { battery: Some(Battery { percent: 101, charging: true }), ..beat.health.clone() },
            Health { macs: vec![[0; 6]; MAX_MACS + 1], ..beat.health.clone() },
            Health { os: "x".repeat(MAX_OS_BYTES + 1), ..beat.health.clone() },
        ] {
            let b = Beat { watching: false, health: bad }.encode();
            assert_eq!(Beat::decode(&b), Err(BeatError::OutOfLimits));
        }
        let packet = magic_packet([1, 2, 3, 4, 5, 6]);
        assert_eq!(&packet[..6], &[0xFF; 6]);
        assert_eq!(&packet[96..], &[1, 2, 3, 4, 5, 6]);
        assert!(wakeable([0x00, 0x1A, 0x2B, 0x3C, 0x4D, 0x5E]));
        assert!(!wakeable([0x02, 0, 0, 0, 0, 1]), "locally administered");
        assert!(!wakeable([0x01, 0, 0x5E, 0, 0, 1]), "multicast");
        assert!(!wakeable([0; 6]) && !wakeable([0xFF; 6]));
        assert!(Health { disk_free: 4 << 30, disk_total: 1 << 40, ..Default::default() }.low_storage());
        assert!(!Health { disk_free: 200 << 30, disk_total: 1 << 40, ..Default::default() }.low_storage());
    }

    proptest! {
        /// Hostile bytes never panic the decoder.
        #[test]
        fn any_bytes_decode_or_fail_cleanly(bytes in proptest::collection::vec(any::<u8>(), 0..1200)) {
            let _ = Beat::decode(&bytes);
        }

        /// Silence only ever makes it more suspicious, and phi is never NaN.
        #[test]
        fn phi_grows_with_silence(gaps in proptest::collection::vec(100u64..20_000, 0..50), a in 0u64..120_000, b in 0u64..120_000) {
            let mut d = Detector::new();
            let mut t = 0;
            d.beat(t);
            for g in gaps { t += g; d.beat(t); }
            let (early, late) = (t + a.min(b), t + a.max(b));
            let (pe, pl) = (d.phi(early), d.phi(late));
            prop_assert!(pe.is_finite() && pl.is_finite());
            prop_assert!(pe <= pl + 1e-9, "{} then {}", pe, pl);
        }
    }
}
