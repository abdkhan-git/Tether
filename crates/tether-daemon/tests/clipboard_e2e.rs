//! End-to-end tests on the real system clipboard. They are `#[ignore]`d:
//! they need an interactive desktop and would overwrite what the person
//! running them has copied (the text is restored afterwards). Run by hand:
//!
//! ```text
//! cargo test -p tether-daemon --release --test clipboard_e2e -- --ignored --nocapture
//! ```
//!
//! Daemon A watches the real clipboard; daemon B uses stdin/stdout, so the
//! two do not share one clipboard. Threat model: SECURITY.md, "Clipboard
//! change events and app privacy marks".

#![cfg(windows)]

mod common;

use std::time::{Duration, Instant};

use common::{pair, settle, Daemon};

struct RestoreClipboard(Option<String>);

impl RestoreClipboard {
    fn save() -> Self {
        Self(arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()))
    }
}

impl Drop for RestoreClipboard {
    fn drop(&mut self) {
        if let (Some(text), Ok(mut c)) = (self.0.take(), arboard::Clipboard::new()) {
            let _ = c.set_text(text);
        }
    }
}

fn copy(text: &str, marks: &[(&str, &[u8])]) {
    tether_os::clipboard::write_text_with_formats(text, marks).expect("write the clipboard");
}

#[test]
#[ignore = "uses the real system clipboard"]
fn copies_sync_private_ones_do_not_and_nothing_echoes() {
    let _restore = RestoreClipboard::save();
    copy("before the daemons start", &[]);
    let mut a = Daemon::on_system_clipboard("alpha");
    assert!(a.wait_for("watching the clipboard mode=change notifications", 0).is_some(), "{}", a.since(0));
    let mut b = Daemon::start("bravo", &[]);
    pair(&mut a, &mut b);
    assert!(!b.since(0).contains("<< before the daemons start"), "what was copied before startup is not a copy");

    // 1. A copy on A arrives at B. Timed from the copy to B printing it.
    let mut lat = Vec::new();
    for i in 0..20 {
        let text = format!("e2e copy {i}");
        let m = b.mark();
        let start = Instant::now();
        copy(&text, &[]);
        b.wait_for_every(&format!("<< {text}"), m, Duration::from_millis(1)).expect("the copy reaches B");
        lat.push(start.elapsed());
        std::thread::sleep(Duration::from_millis(50));
    }
    lat.sort();
    println!("copy on A to printed by B (loopback, 20 copies): p50 {:?}, p95 {:?}, max {:?}", lat[9], lat[18], lat[19]);

    // 2. A copy its app marked private stays on A.
    for (mark, value) in [
        ("ExcludeClipboardContentFromMonitorProcessing", &[0u8][..]),
        ("CanUploadToCloudClipboard", &0u32.to_le_bytes()[..]),
        ("CanIncludeInClipboardHistory", &0u32.to_le_bytes()[..]),
    ] {
        let (ma, mb) = (a.mark(), b.mark());
        copy(&format!("p4ssw0rd marked with {mark}"), &[(mark, value)]);
        assert!(a.wait_for("marked it private", ma).is_some(), "{mark}: A says it kept the copy local");
        settle();
        assert!(!b.since(mb).contains("p4ssw0rd"), "{mark}: a private copy must not reach B");
        assert!(!a.since(ma).contains("broadcasting"), "{mark}: nothing was broadcast");
    }

    // 3. A copy from B lands on A's clipboard and is not sent back.
    let ma = a.mark();
    b.send("written by bravo");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let now = arboard::Clipboard::new().unwrap().get_text().unwrap_or_default();
        if now == "written by bravo" {
            break;
        }
        assert!(Instant::now() < deadline, "B's copy reaches A's clipboard (it holds {now:?})");
        std::thread::sleep(Duration::from_millis(10));
    }
    settle();
    assert!(!a.since(ma).contains("broadcasting"), "A must not echo B's copy back:\n{}", a.since(ma));
}
