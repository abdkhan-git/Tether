//! Knowing when the system clipboard changes, and whether the app that wrote
//! it marked the content private.
//!
//! - Windows: `AddClipboardFormatListener` on a message-only window; the OS
//!   tells us about every change.
//! - macOS: there is no change notification; `NSPasteboard.changeCount` is a
//!   cheap counter, read every 50 ms.
//! - Linux on X11: XFixes selection events for `CLIPBOARD`.
//! - Elsewhere (Wayland, no display): a tick every 100 ms, and the caller
//!   compares contents.
//!
//! Reading the content itself is left to the caller (the daemon uses
//! `arboard`). [`sequence`] lets it check that the content and the marks it
//! read belong to the same copy.

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;
#[cfg(all(unix, not(target_os = "macos")))]
use x11 as imp;

/// Interval for the fallback when the OS offers no change signal.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How changes are being detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The OS reports each change.
    Notifications,
    /// A cheap change counter is read at this interval (macOS).
    ChangeCount(Duration),
    /// Nothing better is available: the caller compares contents at this interval.
    Polling(Duration),
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Notifications => f.write_str("change notifications"),
            Mode::ChangeCount(every) => write!(f, "change counter every {} ms", every.as_millis()),
            Mode::Polling(every) => write!(f, "polling every {} ms", every.as_millis()),
        }
    }
}

/// Watches the clipboard until dropped (or until the callback returns false).
pub struct Watcher {
    mode: Mode,
    stop: Option<Box<dyn FnOnce() + Send>>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    pub(crate) fn new(mode: Mode, stop: Box<dyn FnOnce() + Send>, thread: Option<JoinHandle<()>>) -> Self {
        Self { mode, stop: Some(stop), thread }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Call `on_change` whenever the clipboard may have changed, from a
/// background thread. It returns `false` to stop watching. Falls back to
/// [`poll`] where the OS offers nothing better.
pub fn watch<F: FnMut() -> bool + Send + 'static>(on_change: F) -> io::Result<Watcher> {
    imp::watch(on_change)
}

/// Call `on_change` every `interval`. The fallback when nothing better works.
pub fn poll<F: FnMut() -> bool + Send + 'static>(interval: Duration, mut on_change: F) -> io::Result<Watcher> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new().name("clipboard-poll".into()).spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            std::thread::sleep(interval);
            if flag.load(Ordering::Relaxed) || !on_change() {
                return;
            }
        }
    })?;
    Ok(Watcher::new(Mode::Polling(interval), Box::new(move || stop.store(true, Ordering::Relaxed)), Some(thread)))
}

/// A number that changes whenever the clipboard does, where the OS keeps one
/// (Windows, macOS). Read it before and after reading the content: if it
/// moved, the content and its marks may belong to different copies.
pub fn sequence() -> Option<u64> {
    imp::sequence()
}

/// Put text on the clipboard with extra named formats (privacy marks).
/// Only for tests that check those marks end to end.
#[cfg(windows)]
#[doc(hidden)]
pub use imp::write_text_with_formats;

/// Did the app that wrote the clipboard ask for this copy to stay private
/// (not monitored, not kept in history, not synced)? Password managers do.
/// See SECURITY.md, "Clipboard change events and app privacy marks".
pub fn marked_private() -> bool {
    imp::marked_private()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn polling_ticks_and_stops_when_dropped() {
        let (tx, rx) = mpsc::channel();
        let w = poll(Duration::from_millis(5), move || tx.send(()).is_ok()).unwrap();
        assert_eq!(w.mode(), Mode::Polling(Duration::from_millis(5)));
        for _ in 0..3 {
            rx.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        drop(w); // joins the thread
        while rx.try_recv().is_ok() {}
        std::thread::sleep(Duration::from_millis(30));
        assert!(rx.try_recv().is_err(), "no ticks after drop");
    }

    #[test]
    fn polling_stops_when_the_callback_says_so() {
        let w = poll(Duration::from_millis(1), || false).unwrap();
        drop(w); // would hang if the thread ignored the answer
    }

    #[test]
    fn modes_describe_themselves() {
        assert_eq!(Mode::Notifications.to_string(), "change notifications");
        assert_eq!(Mode::ChangeCount(Duration::from_millis(50)).to_string(), "change counter every 50 ms");
    }
}
