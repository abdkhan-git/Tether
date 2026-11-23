//! Where local text comes from and where received text goes: the system
//! clipboard, or stdin/stdout when running two daemons on one machine (they
//! would share one clipboard otherwise). Console commands (`/pair`, ...) are
//! read from stdin in both modes.

use std::io::BufRead;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, info, warn};

use crate::ctx::Delivered;

/// A line from stdin: a command or (in `--stdin` mode) clipboard text.
pub enum Line {
    Command(String),
    Text(String),
}

/// Polling adds up to this much latency to every copy.
/// TODO(P1): switch to OS change notifications (Windows
/// AddClipboardFormatListener, macOS NSPasteboard changeCount, X11 XFixes).
const POLL_INTERVAL: Duration = Duration::from_millis(100);

fn describe(d: &Delivered) -> Option<String> {
    match d {
        Delivered::Text(_) => None,
        Delivered::Held { from, reason, preview } => {
            Some(format!("held a copy from {from} in history instead of the clipboard ({reason:?}): {preview}"))
        }
        Delivered::KeptLocal(kind) => {
            let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u', 'A', 'E', 'I', 'O', 'U']) { "an" } else { "a" };
            Some(format!("that looks like {article} {kind}; kept on this device and not synced"))
        }
    }
}

pub fn spawn_clipboard(local: UnboundedSender<String>, incoming: Receiver<Delivered>) {
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            let mut clipboard = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    error!(error = %e, "cannot open the system clipboard; try --stdin");
                    return;
                }
            };
            // Whatever is on the clipboard at startup is not a new copy.
            let mut last = clipboard.get_text().unwrap_or_default();
            loop {
                while let Ok(d) = incoming.try_recv() {
                    match d {
                        Delivered::Text(text) => match clipboard.set_text(text.clone()) {
                            // Remember it, so the next poll doesn't echo it back.
                            Ok(()) => last = text,
                            Err(e) => warn!(error = %e, "could not write the clipboard"),
                        },
                        other => {
                            if let Some(msg) = describe(&other) {
                                info!("{msg}");
                            }
                        }
                    }
                }
                if let Ok(text) = clipboard.get_text() {
                    if !text.is_empty() && text != last {
                        last = text.clone();
                        if local.send(text).is_err() {
                            return;
                        }
                    }
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        })
        .expect("spawn clipboard thread");
}

/// Print what peers send to stdout (stdin mode only).
pub fn spawn_stdout(incoming: Receiver<Delivered>) {
    std::thread::Builder::new()
        .name("stdout".into())
        .spawn(move || {
            for d in incoming {
                match &d {
                    Delivered::Text(text) => println!("<< {text}"),
                    other => println!("!! {}", describe(other).unwrap_or_default()),
                }
            }
        })
        .expect("spawn stdout thread");
}

/// Read stdin. Lines starting with `/` are commands; others are clipboard
/// text when `text_mode` is set, and ignored otherwise.
pub fn spawn_stdin(lines: UnboundedSender<Line>, text_mode: bool) {
    std::thread::Builder::new()
        .name("stdin".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { return };
                let line = line.trim_end().to_string();
                let msg = if line.starts_with('/') {
                    Line::Command(line)
                } else if text_mode && !line.is_empty() {
                    Line::Text(line)
                } else {
                    continue;
                };
                if lines.send(msg).is_err() {
                    return;
                }
            }
        })
        .expect("spawn stdin thread");
}
