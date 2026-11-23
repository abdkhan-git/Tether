//! Shared harness for the end-to-end tests: real `tetherd` processes.
//!
//! Daemons run with `--ephemeral` (throwaway keys, nothing written to disk or
//! the OS keychain), `--stdin` (text on stdin/stdout instead of the system
//! clipboard) and `--no-mdns`. With [`Daemon::with_control`] they also open
//! the local control channel, under a unique profile, with a token handed over
//! in `TETHER_CONTROL_TOKEN`.

#![allow(dead_code)] // each test file uses a different part

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tether_ipc::{Endpoint, Token};

pub struct Daemon {
    child: Child,
    stdin: ChildStdin,
    out: Arc<Mutex<String>>,
    pub port: u16,
    pub id: String,
}

/// A control channel opened by a test daemon.
pub struct Control {
    pub profile: String,
    pub endpoint: Endpoint,
    pub token: Token,
}

pub fn unique_profile(name: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    format!("e2e-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn command(name: &str, extra: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tetherd"));
    cmd.args(["--ephemeral", "--stdin", "--no-mdns", "--port", "0", "--name", name])
        .args(extra)
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        .env_remove(tether_ipc::token::TOKEN_ENV);
    cmd
}

impl Daemon {
    pub fn start(name: &str, extra: &[&str]) -> Self {
        Self::spawn(command(name, extra))
    }

    /// Start a daemon with its local control channel open.
    pub fn with_control(name: &str) -> (Self, Control) {
        let profile = unique_profile(name);
        let token = Token::generate().unwrap();
        let mut cmd = command(name, &["--profile", &profile]);
        cmd.env(tether_ipc::token::TOKEN_ENV, token.to_hex().as_str());
        let d = Self::spawn(cmd);
        let endpoint = Endpoint::for_profile(&profile).unwrap();
        (d, Control { profile, endpoint, token })
    }

    /// Run a daemon that is expected to exit (for example, refused at startup).
    pub fn run_to_exit(name: &str, extra: &[&str], token: &Token) -> Output {
        let mut cmd = command(name, extra);
        cmd.env(tether_ipc::token::TOKEN_ENV, token.to_hex().as_str()).stdin(Stdio::null());
        let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("start tetherd");
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        child.wait_with_output().unwrap()
    }

    fn spawn(mut cmd: Command) -> Self {
        let mut child =
            cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("start tetherd");
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
        let started = d.wait_for("tetherd started", 0).unwrap_or_else(|| panic!("daemon starts:\n{}", d.since(0)));
        d.port = field(&started, "port=").parse().expect("port");
        d.id = field(&started, "device=");
        d
    }

    pub fn send(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    pub fn mark(&self) -> usize {
        self.out.lock().unwrap().len()
    }

    pub fn since(&self, mark: usize) -> String {
        self.out.lock().unwrap()[mark..].to_string()
    }

    /// The first line containing `needle` after `from`, waiting up to 10 s.
    pub fn wait_for(&self, needle: &str, from: usize) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(line) = self.since(from).lines().find(|l| l.contains(needle)) {
                return Some(line.to_string());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

pub fn field(line: &str, key: &str) -> String {
    line.split_whitespace().find_map(|w| w.strip_prefix(key)).unwrap_or_default().to_string()
}

pub fn settle() {
    std::thread::sleep(Duration::from_millis(1500));
}

/// Pair `joiner` into `inviter`'s group over the console and wait until they are connected.
pub fn pair(inviter: &mut Daemon, joiner: &mut Daemon) {
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
