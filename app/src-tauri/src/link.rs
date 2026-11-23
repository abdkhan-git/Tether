//! The app's connection to `tetherd`.
//!
//! The app holds no keys and no history and opens no sockets: it asks the
//! daemon over the local control channel, exactly as the CLI does
//! (SECURITY.md rule 9). The channel authenticates both ends, so a program
//! squatting the pipe name gets nothing from the app and cannot feed it data.
//!
//! If no daemon answers for this profile, the app starts the `tetherd` that
//! sits next to its own executable, with `--managed`: that daemon takes no
//! commands on stdin and shuts down cleanly when its stdin closes. The app
//! holds the other end of that pipe, so the daemon it started never outlives
//! it, even if the app crashes. A daemon someone started by hand is used as
//! it is, and left running when the app quits.

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use tether_ipc::{Client, Events, Reply, Request, Response};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::view::LinkState;

/// How long a daemon we started may take to open its control channel
/// (first start creates keys in the credential store and the database).
const START_TIMEOUT: Duration = Duration::from_secs(20);
const START_POLL: Duration = Duration::from_millis(150);
/// How long `shutdown` waits for the daemon to stop by itself before killing it.
const STOP_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Link {
    profile: String,
    /// The connection for requests. A tokio mutex: it is held across the request.
    client: tokio::sync::Mutex<Option<Client>>,
    /// The daemon this app started, if any. A std mutex, never held across an await.
    child: Mutex<Option<Child>>,
    state: watch::Sender<LinkState>,
}

impl Link {
    pub fn new(profile: String) -> Arc<Self> {
        Arc::new(Self {
            profile,
            client: tokio::sync::Mutex::new(None),
            child: Mutex::new(None),
            state: watch::Sender::new(LinkState::Connecting),
        })
    }

    /// Send one request on the shared connection, connecting (and starting
    /// the daemon) first if needed. If the connection turns out to be dead,
    /// `retry` allows one reconnect and a second attempt: only for requests
    /// that are safe to repeat.
    pub async fn call(&self, req: Request, retry: bool) -> Result<Reply, String> {
        let mut slot = self.client.lock().await;
        for attempt in 0..2 {
            if slot.is_none() {
                *slot = Some(self.connect().await.map_err(|e| format!("{e:#}"))?);
            }
            let client = slot.as_mut().expect("connected above");
            match client.call(&req).await {
                Ok(Response::Ok(reply)) => return Ok(reply),
                Ok(Response::Err(refused)) => return Err(refused),
                Err(e) => {
                    *slot = None;
                    if !retry || attempt == 1 {
                        return Err(format!("lost the connection to tetherd: {e:#}"));
                    }
                }
            }
        }
        unreachable!("the loop returns on its second attempt")
    }

    /// Send one request on a connection of its own, for slow requests (joining
    /// can take a minute) that must not hold up the shared one.
    pub async fn call_alone(&self, req: Request) -> Result<Reply, String> {
        let mut client = self.connect().await.map_err(|e| format!("{e:#}"))?;
        match client.call(&req).await.map_err(|e| format!("lost the connection to tetherd: {e:#}"))? {
            Response::Ok(reply) => Ok(reply),
            Response::Err(refused) => Err(refused),
        }
    }

    /// Follow the daemon's events for as long as the app runs, handing each to
    /// `on_event`; reconnect (restarting the daemon if needed) whenever the
    /// stream ends. `on_state` hears about every change in reachability.
    pub async fn run_events(self: Arc<Self>, on_event: impl Fn(tether_ipc::Event), on_state: impl Fn(&LinkState)) {
        let mut delay = Duration::from_millis(250);
        loop {
            match self.subscribe().await {
                Ok(mut events) => {
                    delay = Duration::from_millis(250);
                    self.set_state(LinkState::Connected, &on_state);
                    while let Ok(event) = events.next().await {
                        on_event(event);
                    }
                    warn!("tetherd went away; reconnecting");
                    self.set_state(LinkState::Connecting, &on_state);
                }
                Err(e) => {
                    let reason = format!("{e:#}");
                    warn!(%reason, "cannot reach tetherd");
                    self.set_state(LinkState::Unreachable { reason }, &on_state);
                    delay = (delay * 2).min(Duration::from_secs(5));
                }
            }
            tokio::time::sleep(delay).await;
        }
    }

    fn set_state(&self, state: LinkState, on_state: &impl Fn(&LinkState)) {
        if *self.state.borrow() != state {
            on_state(&state);
            self.state.send_replace(state);
        }
    }

    async fn subscribe(&self) -> Result<Events> {
        self.connect().await?.subscribe().await
    }

    /// An authenticated connection to the profile's daemon, starting one if none answers.
    async fn connect(&self) -> Result<Client> {
        if let Ok(client) = Client::connect(&self.profile).await {
            return Ok(client);
        }
        // Nothing answered. Start ours, unless ours is already starting. (If a
        // daemon someone else started holds the channel but refused us, ours
        // cannot take the channel and stops; that is reported below.)
        self.start_daemon()?;
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            tokio::time::sleep(START_POLL).await;
            match Client::connect(&self.profile).await {
                Ok(client) => return Ok(client),
                Err(e) if Instant::now() >= deadline => {
                    return Err(e.context("tetherd did not open its control channel"))
                }
                Err(_) if !self.ours_is_running() => {
                    bail!("tetherd stopped while starting; see {}", log_path(&self.profile)?.display())
                }
                Err(_) => {}
            }
        }
    }

    fn ours_is_running(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(|p| p.into_inner());
        matches!(child.as_mut().map(|c| c.try_wait()), Some(Ok(None)))
    }

    /// Start our own daemon unless one we started is still running.
    fn start_daemon(&self) -> Result<()> {
        let mut child = self.child.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = child.as_mut() {
            if matches!(c.try_wait(), Ok(None)) {
                return Ok(());
            }
        }
        let exe = daemon_path()?;
        let log = open_log(&self.profile)?;
        let mut cmd = Command::new(&exe);
        cmd.args(["--managed", "--profile", &self.profile])
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(log.try_clone()?)
            .stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // tetherd is a console program; don't flash a console window.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let started = cmd.spawn().with_context(|| format!("starting {}", exe.display()))?;
        info!(pid = started.id(), "started tetherd");
        *child = Some(started);
        Ok(())
    }

    /// Stop the daemon this app started, if any: close its stdin so it shuts
    /// down cleanly (closing its connections and removing its control token),
    /// and kill it only if it has not stopped in time.
    pub fn shutdown(&self) {
        let Some(mut child) = self.child.lock().unwrap_or_else(|p| p.into_inner()).take() else { return };
        drop(child.stdin.take());
        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        warn!("tetherd did not stop in time; killing it");
        let _ = child.kill();
    }
}

/// The `tetherd` next to this executable: the bundle puts it there, and so
/// does a workspace build (`target/debug`).
fn daemon_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding the app's own executable")?;
    let path = exe.with_file_name(format!("tetherd{}", std::env::consts::EXE_SUFFIX));
    if !path.is_file() {
        return Err(anyhow!(
            "tetherd was not found at {} (build it with `cargo build -p tether-daemon`)",
            path.display()
        ));
    }
    Ok(path)
}

/// The daemon's log: `Tether/logs/tetherd-<profile>.log` in the local data
/// folder, replaced at each start (the previous one is kept beside it). It
/// holds what tetherd logs: device ids, addresses and events, never clipboard
/// text or keys.
pub fn log_path(profile: &str) -> Result<PathBuf> {
    let base = dirs::data_local_dir().context("finding the local data folder")?;
    Ok(base.join("Tether").join("logs").join(format!("tetherd-{profile}.log")))
}

fn open_log(profile: &str) -> Result<File> {
    let path = log_path(profile)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    if path.exists() {
        let _ = fs::rename(&path, path.with_extension("previous.log"));
    }
    File::create(&path).with_context(|| format!("creating {}", path.display()))
}
