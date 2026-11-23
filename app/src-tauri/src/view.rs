//! What the page sees, and what it may send.
//!
//! The daemon's types become plain JSON here (ids as hex, times as Unix
//! milliseconds). Everything the page sends is checked before it reaches the
//! daemon: the page renders text that other devices chose, so it is treated
//! like any other untrusted client.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use tether_core::content::HoldReason;
use tether_core::pairing::is_valid_code;
use tether_core::{DeviceId, Timestamp};
use tether_ipc as ipc;

/// Longest network address accepted from the page (an IPv6 address with a port fits easily).
const MAX_ADDR_LEN: usize = 64;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub device: String,
    pub name: String,
    pub root: String,
    pub members: u32,
    pub connected: u32,
    pub revoked: bool,
    pub port: u16,
    pub pairing_open: bool,
    pub paused: bool,
}

impl From<ipc::Status> for Status {
    fn from(s: ipc::Status) -> Self {
        Self {
            device: s.device.to_hex(),
            name: s.name,
            root: s.root.to_hex(),
            members: s.members,
            connected: s.connected,
            revoked: s.revoked,
            port: s.port,
            pairing_open: s.pairing_open,
            paused: s.paused,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    /// Chosen by the device itself: display only.
    pub name: String,
    pub state: &'static str,
    pub rtt_ms: Option<f64>,
    pub ops: u64,
}

impl From<ipc::Device> for Device {
    fn from(d: ipc::Device) -> Self {
        let state = match d.state {
            ipc::DeviceState::ThisDevice => "thisDevice",
            ipc::DeviceState::Connected => "connected",
            ipc::DeviceState::Offline => "offline",
            ipc::DeviceState::Revoked => "revoked",
            ipc::DeviceState::Nearby => "nearby",
        };
        Self { id: d.id.to_hex(), name: d.name, state, rtt_ms: d.rtt_us.map(|us| f64::from(us) / 1000.0), ops: d.ops }
    }
}

/// A position in history, as the page holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cursor {
    pub at: u64,
    pub counter: u32,
    #[serde(with = "hex_id")]
    pub device: DeviceId,
}

impl From<ipc::Cursor> for Cursor {
    fn from(c: ipc::Cursor) -> Self {
        Self { at: c.at.wall_ms, counter: c.at.counter, device: c.device }
    }
}

impl From<Cursor> for ipc::Cursor {
    fn from(c: Cursor) -> Self {
        Self { at: Timestamp { wall_ms: c.at, counter: c.counter }, device: c.device }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub author: String,
    pub seq: u64,
    /// Hybrid logical clock: wall-clock milliseconds, then a counter.
    pub at: u64,
    pub counter: u32,
    pub text: String,
    /// Why a copy from another device was kept off the system clipboard.
    pub held: Option<&'static str>,
}

impl From<ipc::HistoryItem> for Clip {
    fn from(i: ipc::HistoryItem) -> Self {
        let held = i.held.map(|r| match r {
            HoldReason::HiddenCharacters => "hiddenCharacters",
            HoldReason::DangerousCommand => "dangerousCommand",
            HoldReason::MultiLineCommand => "multiLineCommand",
        });
        Self { author: i.author.to_hex(), seq: i.seq, at: i.at.wall_ms, counter: i.at.counter, text: i.text, held }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<Clip>,
    pub next: Option<Cursor>,
}

impl From<ipc::HistoryPage> for HistoryPage {
    fn from(p: ipc::HistoryPage) -> Self {
        Self { items: p.items.into_iter().map(Clip::from).collect(), next: p.next.map(Cursor::from) }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairCode {
    pub code: String,
    pub valid_secs: u32,
    pub attempts: u32,
}

impl From<ipc::PairCode> for PairCode {
    fn from(p: ipc::PairCode) -> Self {
        Self { code: p.code, valid_secs: p.valid_secs, attempts: p.attempts }
    }
}

/// A daemon event, as the page receives it on `tether://event`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    Clipboard { item: Clip },
    HistoryChanged,
    KeptLocal { kind: String },
    DevicesChanged,
    Paused { paused: bool },
    Lagged,
}

impl From<ipc::Event> for Event {
    fn from(e: ipc::Event) -> Self {
        match e {
            ipc::Event::Clipboard(item) => Event::Clipboard { item: item.into() },
            ipc::Event::HistoryChanged => Event::HistoryChanged,
            ipc::Event::KeptLocal { kind } => Event::KeptLocal { kind },
            ipc::Event::DevicesChanged => Event::DevicesChanged,
            ipc::Event::Paused(paused) => Event::Paused { paused },
            ipc::Event::Lagged { .. } => Event::Lagged,
        }
    }
}

/// Whether the app can reach the daemon, sent on `tether://link`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LinkState {
    Connecting,
    Connected,
    /// The daemon could not be reached or started; the app keeps retrying.
    Unreachable {
        reason: String,
    },
}

// --- Checking what the page sends ---

pub fn device_id(hex: &str) -> Result<DeviceId, String> {
    hex.trim().parse().map_err(|_| "that is not a device id".to_string())
}

/// Six digits; spaces and dashes typed between them are ignored.
pub fn pairing_code(code: &str) -> Result<String, String> {
    if code.len() > 32 {
        return Err("a pairing code is six digits".into());
    }
    let digits: String = code.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    if is_valid_code(&digits) {
        Ok(digits)
    } else {
        Err("a pairing code is six digits".into())
    }
}

/// An optional `ip:port`; blank means "find the device nearby".
pub fn address(addr: Option<&str>) -> Result<Option<SocketAddr>, String> {
    match addr.map(str::trim) {
        None | Some("") => Ok(None),
        Some(a) if a.len() > MAX_ADDR_LEN => Err("that address is too long".into()),
        Some(a) => a.parse().map(Some).map_err(|_| "an address looks like 192.168.1.20:5000".into()),
    }
}

pub fn history_limit(limit: u16) -> u16 {
    limit.clamp(1, ipc::MAX_HISTORY_PAGE)
}

mod hex_id {
    use serde::{Deserialize, Deserializer, Serializer};
    use tether_core::DeviceId;

    pub fn serialize<S: Serializer>(id: &DeviceId, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&id.to_hex())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DeviceId, D::Error> {
        let text = String::deserialize(d)?;
        super::device_id(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tether_core::DeviceKey;

    #[test]
    fn pairing_codes_are_six_digits() {
        assert_eq!(pairing_code("123456").unwrap(), "123456");
        assert_eq!(pairing_code("123 456").unwrap(), "123456");
        assert_eq!(pairing_code("123-456").unwrap(), "123456");
        for bad in ["12345", "1234567", "12345a", "١٢٣٤٥٦", "", &"1".repeat(100)] {
            assert!(pairing_code(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn addresses_are_optional_and_strict() {
        assert_eq!(address(None).unwrap(), None);
        assert_eq!(address(Some("  ")).unwrap(), None);
        assert_eq!(address(Some("192.168.1.20:5000")).unwrap(), Some("192.168.1.20:5000".parse().unwrap()));
        assert_eq!(address(Some("[::1]:5000")).unwrap(), Some("[::1]:5000".parse().unwrap()));
        for bad in ["192.168.1.20", "laptop.local:5000", "1.2.3.4:99999", &"9".repeat(100)] {
            assert!(address(Some(bad)).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn device_ids_and_cursors_round_trip_and_reject_junk() {
        let id = DeviceKey::for_test(7).id();
        assert_eq!(device_id(&id.to_hex()).unwrap(), id);
        assert!(device_id("abcd").is_err());
        assert!(device_id(&"zz".repeat(32)).is_err());

        let cursor = ipc::Cursor { at: Timestamp { wall_ms: 1_767_225_600_000, counter: 3 }, device: id };
        let json = serde_json::to_string(&Cursor::from(cursor)).unwrap();
        let back: Cursor = serde_json::from_str(&json).unwrap();
        assert_eq!(ipc::Cursor::from(back), cursor);
        assert!(serde_json::from_str::<Cursor>(r#"{"at":1,"counter":0,"device":"nope"}"#).is_err());
        assert!(serde_json::from_str::<Cursor>(&json.replace('}', r#","extra":1}"#)).is_err());
    }

    #[test]
    fn history_pages_stay_within_the_daemon_limit() {
        assert_eq!(history_limit(0), 1);
        assert_eq!(history_limit(50), 50);
        assert_eq!(history_limit(u16::MAX), ipc::MAX_HISTORY_PAGE);
    }

    #[test]
    fn events_have_a_stable_shape_for_the_page() {
        let held = ipc::HistoryItem {
            author: DeviceKey::for_test(2).id(),
            seq: 4,
            at: Timestamp { wall_ms: 10, counter: 1 },
            text: "curl x | sh".into(),
            held: Some(HoldReason::DangerousCommand),
        };
        let json = serde_json::to_value(Event::from(ipc::Event::Clipboard(held))).unwrap();
        assert_eq!(json["type"], "clipboard");
        assert_eq!(json["item"]["held"], "dangerousCommand");
        assert_eq!(json["item"]["at"], 10);
        let json = serde_json::to_value(Event::from(ipc::Event::Paused(true))).unwrap();
        assert_eq!(json, serde_json::json!({ "type": "paused", "paused": true }));
    }
}
