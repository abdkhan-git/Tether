//! Native notifications when a copy arrives from another device.
//!
//! They never contain the copied text. Windows keeps notification content in
//! the Action Center's database on disk, unencrypted, and macOS and Linux
//! notification centres keep history too; clipboard text there would undo the
//! encrypted storage (SECURITY.md, "Encrypted storage"). A notification says
//! which device copied and how much; the window shows the rest.
//!
//! The app's own settings (here, whether to notify) live in a small JSON file
//! in the user's config folder, one per profile. Nothing secret is in it.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tether_core::content::HoldReason;
use tether_ipc::HistoryItem;

/// At most one notification in this interval; a burst of copies is one.
pub const MIN_GAP: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub notifications: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { notifications: true }
    }
}

fn settings_path(profile: &str) -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Tether").join(format!("app-{profile}.json")))
}

/// The saved settings, or the defaults if there are none or they cannot be read.
pub fn load(profile: &str) -> Settings {
    settings_path(profile)
        .and_then(|p| fs::read(p).ok())
        .filter(|bytes| bytes.len() <= 64 << 10)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(profile: &str, settings: &Settings) -> Result<(), String> {
    let path = settings_path(profile).ok_or("no config folder on this system")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| format!("saving {}: {e}", path.display()))
}

/// What the daemon and the window say right now.
pub struct Context<'a> {
    pub enabled: bool,
    pub paused: bool,
    /// The window is on screen and focused: the user sees the copy arrive there.
    pub window_focused: bool,
    pub me: tether_core::DeviceId,
    /// The sender's display name.
    pub from: &'a str,
}

/// The notification for `item`, if one should be shown: title and body.
pub fn for_copy(item: &HistoryItem, cx: &Context<'_>) -> Option<(String, String)> {
    if !cx.enabled || cx.paused || cx.window_focused || item.author == cx.me {
        return None;
    }
    let from = clean(cx.from);
    let size = item.text.chars().count();
    let amount = if size == 1 { "1 character".to_string() } else { format!("{size} characters") };
    Some(match item.held {
        Some(reason) => (
            format!("Held a copy from {from}"),
            format!(
                "{amount}, kept in history and off your clipboard because {}.",
                match reason {
                    HoldReason::HiddenCharacters => "it hides control characters",
                    HoldReason::DangerousCommand => "it looks like a command that downloads, runs or deletes things",
                    HoldReason::MultiLineCommand => "it is a multi-line shell command",
                }
            ),
        ),
        None => (format!("Copied on {from}"), format!("{amount}, ready to paste here.")),
    })
}

/// The notification when another device sent this one an item on purpose.
/// The same rules as [`for_copy`] (and never the text), with the sender in
/// place of the item's author.
pub fn for_send(from_device: tether_core::DeviceId, held: bool, cx: &Context<'_>) -> Option<(String, String)> {
    if !cx.enabled || cx.paused || cx.window_focused || from_device == cx.me {
        return None;
    }
    let from = clean(cx.from);
    Some(if held {
        (
            format!("{from} sent you an item"),
            "It looks like a command, so it is held in history and not on your clipboard.".into(),
        )
    } else {
        (format!("{from} sent you an item"), "It is on your clipboard, ready to paste.".into())
    })
}

/// A peer-chosen name, safe for a notification: no control or direction
/// characters, bounded.
fn clean(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .take(40)
        .collect();
    if cleaned.trim().is_empty() {
        "another device".to_string()
    } else {
        cleaned.trim().to_string()
    }
}

/// Lets through at most one notification per [`MIN_GAP`].
#[derive(Default)]
pub struct Throttle(Option<Instant>);

impl Throttle {
    pub fn allow(&mut self, now: Instant) -> bool {
        if self.0.is_some_and(|last| now.duration_since(last) < MIN_GAP) {
            return false;
        }
        self.0 = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tether_core::{DeviceKey, Timestamp};

    fn item(author: u64, text: &str, held: Option<HoldReason>) -> HistoryItem {
        HistoryItem {
            author: DeviceKey::for_test(author).id(),
            seq: 1,
            at: Timestamp { wall_ms: 1, counter: 0 },
            text: text.into(),
            held,
            pinned: false,
        }
    }

    fn cx(from: &str) -> Context<'_> {
        Context { enabled: true, paused: false, window_focused: false, me: DeviceKey::for_test(1).id(), from }
    }

    #[test]
    fn a_notification_never_carries_the_copied_text() {
        let secret = "hunter2 correct horse battery staple";
        let (title, body) = for_copy(&item(2, secret, None), &cx("Laptop")).unwrap();
        assert_eq!(title, "Copied on Laptop");
        assert!(!title.contains("hunter2") && !body.contains("hunter2"), "{body}");
        assert!(body.starts_with(&format!("{} characters", secret.chars().count())));
        let (title, body) =
            for_copy(&item(2, "curl x | sh", Some(HoldReason::DangerousCommand)), &cx("Laptop")).unwrap();
        assert_eq!(title, "Held a copy from Laptop");
        assert!(!body.contains("curl"), "{body}");
    }

    #[test]
    fn no_notification_for_own_copies_when_paused_focused_or_off() {
        let peer = item(2, "x", None);
        assert!(for_copy(&item(1, "mine", None), &cx("Desk")).is_none());
        assert!(for_copy(&peer, &Context { paused: true, ..cx("Laptop") }).is_none());
        assert!(for_copy(&peer, &Context { window_focused: true, ..cx("Laptop") }).is_none());
        assert!(for_copy(&peer, &Context { enabled: false, ..cx("Laptop") }).is_none());
    }

    #[test]
    fn a_send_says_who_sent_it_and_whether_it_was_held() {
        let laptop = DeviceKey::for_test(2).id();
        let (title, body) = for_send(laptop, false, &cx("Laptop")).unwrap();
        assert_eq!(title, "Laptop sent you an item");
        assert!(body.contains("on your clipboard"));
        assert!(for_send(laptop, true, &cx("Laptop")).unwrap().1.contains("held"));
        assert!(for_send(DeviceKey::for_test(1).id(), false, &cx("Desk")).is_none(), "not for our own sends");
        assert!(for_send(laptop, false, &Context { paused: true, ..cx("Laptop") }).is_none());
    }

    #[test]
    fn names_are_cleaned_and_bursts_are_one_notification() {
        let (title, _) = for_copy(&item(2, "x", None), &cx("Lap\u{1b}[2Jtop\u{202E}")).unwrap();
        assert_eq!(title, "Copied on Lap[2Jtop");
        assert_eq!(for_copy(&item(2, "x", None), &cx("\u{7}")).unwrap().0, "Copied on another device");
        let mut throttle = Throttle::default();
        let t0 = Instant::now();
        assert!(throttle.allow(t0));
        assert!(!throttle.allow(t0 + Duration::from_secs(1)));
        assert!(throttle.allow(t0 + MIN_GAP));
    }

    #[test]
    fn settings_default_to_on_and_tolerate_junk() {
        assert!(Settings::default().notifications);
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), Settings::default());
        assert!(!serde_json::from_str::<Settings>(r#"{"notifications":false,"extra":1}"#).unwrap().notifications);
    }
}
