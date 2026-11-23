//! Where local text comes from and where received text goes: the system
//! clipboard, or stdin/stdout when running two daemons on one machine
//! (they would share one clipboard otherwise).

use std::io::BufRead;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, warn};

/// Polling adds up to this much latency to every copy.
/// TODO(stage 1): switch to OS change notifications (Windows
/// AddClipboardFormatListener, macOS NSPasteboard changeCount, X11 XFixes).
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub fn spawn_clipboard(local: UnboundedSender<String>, incoming: Receiver<String>) {
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
                while let Ok(text) = incoming.try_recv() {
                    match clipboard.set_text(text.clone()) {
                        // Remember it, so the next poll doesn't echo it back.
                        Ok(()) => last = text,
                        Err(e) => warn!(error = %e, "could not write the clipboard"),
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

pub fn spawn_stdin(local: UnboundedSender<String>, incoming: Receiver<String>) {
    std::thread::Builder::new()
        .name("stdout".into())
        .spawn(move || {
            for text in incoming {
                println!("<< {text}");
            }
        })
        .expect("spawn stdout thread");
    std::thread::Builder::new()
        .name("stdin".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { return };
                if !line.is_empty() && local.send(line).is_err() {
                    return;
                }
            }
        })
        .expect("spawn stdin thread");
}
