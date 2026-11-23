//! macOS has no clipboard change notification. `changeCount` is a cheap
//! counter that moves on every write, so a 50 ms read of it costs almost
//! nothing and the caller only reads content when it actually changed.
//! No `unsafe` here: these AppKit calls are safe in `objc2-app-kit`.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use objc2_app_kit::NSPasteboard;

use super::{Mode, Watcher};

const INTERVAL: Duration = Duration::from_millis(50);

/// Pasteboard types that the nspasteboard.org convention (used by
/// 1Password, KeePassXC, Bitwarden and others) defines for private copies.
const PRIVATE_TYPES: [&str; 2] = ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"];

pub fn watch<F: FnMut() -> bool + Send + 'static>(mut on_change: F) -> io::Result<Watcher> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new().name("clipboard-watch".into()).spawn(move || {
        let pasteboard = NSPasteboard::generalPasteboard();
        let mut last = pasteboard.changeCount();
        while !flag.load(Ordering::Relaxed) {
            std::thread::sleep(INTERVAL);
            let now = pasteboard.changeCount();
            if now != last {
                last = now;
                if !on_change() {
                    return;
                }
            }
        }
    })?;
    Ok(Watcher::new(Mode::ChangeCount(INTERVAL), Box::new(move || stop.store(true, Ordering::Relaxed)), Some(thread)))
}

pub fn sequence() -> Option<u64> {
    Some(NSPasteboard::generalPasteboard().changeCount() as u64)
}

pub fn marked_private() -> bool {
    let Some(types) = NSPasteboard::generalPasteboard().types() else { return false };
    types.iter().any(|t| PRIVATE_TYPES.contains(&t.to_string().as_str()))
}
