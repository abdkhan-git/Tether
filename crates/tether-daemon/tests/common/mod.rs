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
    stdin: Option<ChildStdin>,
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

/// How a test daemon runs. The default is what almost every test wants.
struct Opts {
    /// Throwaway keys, nothing on disk or in the credential store.
    ephemeral: bool,
    /// Text on stdin/stdout instead of the system clipboard.
    stdin: bool,
    port: u16,
}

impl Default for Opts {
    fn default() -> Self {
        Self { ephemeral: true, stdin: true, port: 0 }
    }
}

fn build(name: &str, opts: Opts, extra: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tetherd"));
    if opts.stdin {
        cmd.arg("--stdin");
    }
    if opts.ephemeral {
        cmd.arg("--ephemeral");
    }
    cmd.args(["--no-mdns", "--port", &opts.port.to_string(), "--name", name])
        .args(extra)
        .env("RUST_LOG", "info,tetherd::net=debug")
        .env("NO_COLOR", "1")
        .env_remove(tether_ipc::token::TOKEN_ENV);
    cmd
}

fn command(name: &str, extra: &[&str]) -> Command {
    build(name, Opts::default(), extra)
}

impl Daemon {
    pub fn start(name: &str, extra: &[&str]) -> Self {
        Self::spawn(command(name, extra))
    }

    /// Start a daemon on the real system clipboard (no `--stdin`). Its stdin
    /// still takes console commands. Only for `#[ignore]`d tests.
    pub fn on_system_clipboard(name: &str) -> Self {
        Self::spawn(build(name, Opts { stdin: false, ..Opts::default() }, &[]))
    }

    /// Start a daemon that keeps its identity and data under `profile`, in
    /// the real credential store and data folder, on a fixed `port`.
    /// Only for `#[ignore]`d tests, which must clean up with [`forget_profile`].
    pub fn persistent(name: &str, profile: &str, port: u16, extra: &[&str]) -> Self {
        let mut args = vec!["--profile", profile];
        args.extend_from_slice(extra);
        Self::spawn(build(name, Opts { ephemeral: false, port, ..Opts::default() }, &args))
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

    /// Start a daemon as the desktop app does (`--managed`, which rules out
    /// `--stdin`), with its control channel open. It watches the system
    /// clipboard (read only; no peer ever writes to it in these tests). On a
    /// machine with no clipboard it logs an error and runs on.
    pub fn managed(name: &str) -> (Self, Control) {
        let profile = unique_profile(name);
        let token = Token::generate().unwrap();
        let mut cmd = build(name, Opts { stdin: false, ..Opts::default() }, &["--managed", "--profile", &profile]);
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
        let stdin = child.stdin.take();
        let mut d = Daemon { child, stdin, out, port: 0, id: String::new() };
        let started = d.wait_for("tetherd started", 0).unwrap_or_else(|| panic!("daemon starts:\n{}", d.since(0)));
        d.port = field(&started, "port=").parse().expect("port");
        d.id = field(&started, "device=");
        d
    }

    pub fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    }

    /// Close the daemon's stdin, as an app that quits or crashes does, and
    /// wait up to `limit` for it to exit.
    pub fn close_stdin_and_wait(&mut self, limit: Duration) -> Option<std::process::ExitStatus> {
        drop(self.stdin.take());
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        None
    }

    pub fn mark(&self) -> usize {
        self.out.lock().unwrap().len()
    }

    pub fn since(&self, mark: usize) -> String {
        self.out.lock().unwrap()[mark..].to_string()
    }

    /// The first line containing `needle` after `from`, waiting up to 10 s.
    pub fn wait_for(&self, needle: &str, from: usize) -> Option<String> {
        self.wait_for_every(needle, from, Duration::from_millis(50))
    }

    /// As `wait_for`, checking every `step` (fine-grained, for timing).
    pub fn wait_for_every(&self, needle: &str, from: usize, step: Duration) -> Option<String> {
        self.wait_for_within(needle, from, step, Duration::from_secs(10))
    }

    /// As `wait_for`, checking every `step` for up to `limit`.
    pub fn wait_for_within(&self, needle: &str, from: usize, step: Duration, limit: Duration) -> Option<String> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if let Some(line) = self.since(from).lines().find(|l| l.contains(needle)) {
                return Some(line.to_string());
            }
            std::thread::sleep(step);
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

/// Delete everything a persistent test daemon left: its three credentials and its data folder.
pub fn forget_profile(profile: &str) {
    for kind in ["device-key", "db-key", "control-token"] {
        if let Ok(entry) = keyring::Entry::new("Tether", &format!("{kind}/{profile}")) {
            let _ = entry.delete_credential();
        }
    }
    if let Some(base) = dirs::data_local_dir() {
        let _ = std::fs::remove_dir_all(base.join("Tether").join(profile));
    }
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
