//! Linux and other Unix: XFixes tells us when the `CLIPBOARD` selection
//! changes owner (every copy). Under Wayland, or without a display, there is
//! no portable signal, so we fall back to polling. Pure Rust (x11rb), no `unsafe`.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use x11rb::connection::Connection;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use super::{poll, Mode, Watcher, POLL_INTERVAL};

type Setup = Result<RustConnection, Box<dyn std::error::Error + Send + Sync>>;

fn subscribe() -> Setup {
    let (conn, screen) = x11rb::connect(None)?;
    conn.xfixes_query_version(5, 0)?.reply()?;
    let root = conn.setup().roots[screen].root;
    let clipboard = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
    let mask = SelectionEventMask::SET_SELECTION_OWNER
        | SelectionEventMask::SELECTION_WINDOW_DESTROY
        | SelectionEventMask::SELECTION_CLIENT_CLOSE;
    conn.xfixes_select_selection_input(root, clipboard, mask)?;
    conn.flush()?;
    Ok(conn)
}

pub fn watch<F: FnMut() -> bool + Send + 'static>(on_change: F) -> io::Result<Watcher> {
    // A Wayland session's clipboard is not the X11 selection; XFixes would
    // only see X11 apps. Poll there instead.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_none() {
        return poll(POLL_INTERVAL, on_change);
    }
    let (ready_tx, ready_rx) = mpsc::sync_channel::<bool>(1);
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let (fallback_tx, fallback_rx) = mpsc::sync_channel::<F>(1);
    std::thread::Builder::new().name("clipboard-watch".into()).spawn(move || {
        let mut callback = on_change;
        let conn = match subscribe() {
            Ok(conn) => conn,
            Err(_) => {
                let _ = ready_tx.send(false);
                let _ = fallback_tx.send(callback); // hand it back for polling
                return;
            }
        };
        let _ = ready_tx.send(true);
        while let Ok(event) = conn.wait_for_event() {
            if flag.load(Ordering::Relaxed) {
                return;
            }
            if let Event::XfixesSelectionNotify(_) = event {
                if !callback() {
                    return;
                }
            }
        }
    })?;
    if ready_rx.recv().unwrap_or(false) {
        // The thread blocks in wait_for_event and cannot be woken; it exits
        // at the next clipboard event after the watcher is dropped.
        Ok(Watcher::new(Mode::Notifications, Box::new(move || stop.store(true, Ordering::Relaxed)), None))
    } else {
        let callback = fallback_rx.recv().map_err(|_| io::Error::other("the clipboard watcher thread stopped"))?;
        poll(POLL_INTERVAL, callback)
    }
}

pub fn sequence() -> Option<u64> {
    None
}

/// Not supported yet (KDE's `x-kde-passwordManagerHint` is the candidate).
pub fn marked_private() -> bool {
    false
}
