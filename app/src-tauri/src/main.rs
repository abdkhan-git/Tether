//! The Tether desktop app: the window and the tray. A thin client of `tetherd`
//! (see `link`); it holds no keys, stores nothing, and opens no sockets.
//! Threat model: docs/SECURITY.md, "Desktop app".
//!
//! What the page can do is exactly the commands below, each granted to the
//! main window by capabilities/main.json (build.rs makes ungranted commands
//! fail). The window can only show the bundled page: navigation anywhere
//! else, and new windows, are refused.

// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod guard;
mod link;
mod login;
mod tray;
mod view;

use std::sync::Arc;

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tether_ipc::{Reply, Request};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::link::Link;

use crate::login::DEFAULT_PROFILE;
/// Events from the daemon, as `view::Event`.
const EVENT: &str = "tether://event";
/// Whether the daemon can be reached, as `view::LinkState`.
const LINK: &str = "tether://link";

type Res<T> = Result<T, String>;

fn unexpected(reply: Reply) -> String {
    format!("unexpected reply from tetherd: {reply:?}")
}

// --- Commands (the page's entire API) ---

#[tauri::command]
async fn status(link: State<'_, Arc<Link>>) -> Res<view::Status> {
    match link.call(Request::Status, true).await? {
        Reply::Status(s) => Ok(s.into()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn devices(link: State<'_, Arc<Link>>) -> Res<Vec<view::Device>> {
    match link.call(Request::Devices, true).await? {
        Reply::Devices(d) => Ok(d.into_iter().map(view::Device::from).collect()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn history(link: State<'_, Arc<Link>>, before: Option<view::Cursor>, limit: u16) -> Res<view::HistoryPage> {
    let req = Request::History { before: before.map(Into::into), limit: view::history_limit(limit) };
    match link.call(req, true).await? {
        Reply::History(page) => Ok(page.into()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn pair(link: State<'_, Arc<Link>>) -> Res<view::PairCode> {
    match link.call(Request::Pair, false).await? {
        Reply::PairCode(p) => Ok(p.into()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn join(link: State<'_, Arc<Link>>, code: String, addr: Option<String>) -> Res<String> {
    let req = Request::Join { code: view::pairing_code(&code)?, addr: view::address(addr.as_deref())? };
    match link.call_alone(req).await? {
        Reply::Joined { device } => Ok(device.to_hex()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn revoke(link: State<'_, Arc<Link>>, device: String) -> Res<()> {
    match link.call(Request::Revoke { device: view::device_id(&device)? }, false).await? {
        Reply::Revoked => Ok(()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn set_pinned(link: State<'_, Arc<Link>>, author: String, seq: u64, pinned: bool) -> Res<()> {
    match link.call(Request::SetPinned { author: view::device_id(&author)?, seq, pinned }, true).await? {
        Reply::Marked => Ok(()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn delete_item(link: State<'_, Arc<Link>>, author: String, seq: u64) -> Res<()> {
    match link.call(Request::Delete { author: view::device_id(&author)?, seq }, false).await? {
        Reply::Marked => Ok(()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn set_paused(link: State<'_, Arc<Link>>, paused: bool) -> Res<bool> {
    match link.call(Request::SetPaused { paused }, true).await? {
        Reply::Paused(p) => Ok(p),
        r => Err(unexpected(r)),
    }
}

/// Open a web or email link in the user's own browser or mail app. Only on
/// a click in the page (a local user action), and only links `guard` accepts.
#[tauri::command]
fn open_link(url: String) -> Res<()> {
    let url = guard::external_link(&url)?;
    open::that_detached(url.as_str()).map_err(|e| format!("could not open the link: {e}"))
}

/// This run's login entry (see `login`).
struct LoginEntry(Result<auto_launch::AutoLaunch, String>);

/// Whether Tether opens when the user signs in to this computer.
#[tauri::command]
fn autostart(entry: State<'_, LoginEntry>) -> Res<bool> {
    let entry = entry.0.as_ref().map_err(Clone::clone)?;
    entry.is_enabled().map_err(|e| format!("could not read the login setting: {e}"))
}

/// Turn opening at sign-in on or off. Off until the user turns it on here.
#[tauri::command]
fn set_autostart(entry: State<'_, LoginEntry>, enabled: bool) -> Res<bool> {
    let entry = entry.0.as_ref().map_err(Clone::clone)?;
    let result = if enabled { entry.enable() } else { entry.disable() };
    result.map_err(|e| format!("could not change the login setting: {e}"))?;
    info!(enabled, "open at sign-in changed");
    entry.is_enabled().map_err(|e| format!("could not read the login setting: {e}"))
}

// --- Startup ---

/// How this run was started.
#[derive(Debug, PartialEq, Eq)]
struct Launch {
    /// The daemon to talk to (`--profile NAME`, as for tetherd and the CLI).
    profile: String,
    /// Started at sign-in: stay in the tray until the user opens the window.
    hidden: bool,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Launch, String> {
    let mut args = args.into_iter();
    let mut launch = Launch { profile: DEFAULT_PROFILE.to_string(), hidden: false };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" => launch.profile = args.next().ok_or("--profile needs a name")?,
            "--hidden" => launch.hidden = true,
            _ => {}
        }
    }
    let p = &launch.profile;
    let valid =
        !p.is_empty() && p.len() <= 32 && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if valid {
        Ok(launch)
    } else {
        Err("--profile may only contain letters, digits, '-' and '_' (at most 32)".into())
    }
}

fn main_window(app: &AppHandle, hidden: bool) -> tauri::Result<()> {
    let dev = tauri::is_dev();
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Tether")
        .visible(!hidden)
        // A start at sign-in stays in the tray: no focus taken from what the user is doing.
        .focused(!hidden)
        .inner_size(1240.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .center()
        .on_navigation(move |url| {
            let ok = guard::is_app_url(url, dev);
            if !ok {
                // The URL itself may have come from clipboard text: not logged.
                warn!("refused a navigation away from the app");
            }
            ok
        })
        .on_new_window(|_, _| {
            warn!("refused to open a new window");
            NewWindowResponse::Deny
        })
        .disable_drag_drop_handler()
        .build()?;
    // Closing the window hides it; Tether keeps running in the tray.
    let hide = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hide.hide();
        }
    });
    Ok(())
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let Launch { profile, hidden } = match parse_args(std::env::args().skip(1)) {
        Ok(launch) => launch,
        Err(e) => {
            eprintln!("tether-app: {e}");
            std::process::exit(2);
        }
    };
    let link = Link::new(profile.clone());
    let events_link = link.clone();

    let mut builder = tauri::Builder::default();
    if profile == DEFAULT_PROFILE {
        // A second launch shows this window instead of adding a second tray icon.
        // Other profiles (two apps on one machine, for testing) skip this.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_window(app)));
    }
    let app = builder
        .manage(LoginEntry(login::entry(&profile)))
        .manage(link.clone())
        .invoke_handler(tauri::generate_handler![
            status,
            devices,
            history,
            pair,
            join,
            revoke,
            set_paused,
            set_pinned,
            delete_item,
            open_link,
            autostart,
            set_autostart
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            main_window(&handle, hidden)?;
            let tray = tray::build(&handle)?;
            app.manage(tray);
            info!(%profile, "Tether started");

            let (events_to, state_to) = (handle.clone(), handle.clone());
            tauri::async_runtime::spawn(events_link.run_events(
                move |event| {
                    if matches!(event, tether_ipc::Event::Paused(_) | tether_ipc::Event::DevicesChanged) {
                        tray::refresh(&events_to);
                    }
                    let _ = events_to.emit_to("main", EVENT, view::Event::from(event));
                },
                move |state| {
                    tray::refresh(&state_to);
                    let _ = state_to.emit_to("main", LINK, state.clone());
                },
            ));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("building the Tether app");

    app.run(move |_app, event| {
        if let RunEvent::Exit = event {
            // Stop the daemon we started (not one someone else started).
            link.shutdown();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn launch_arguments() {
        assert_eq!(parse_args(args(&[])).unwrap(), Launch { profile: "default".into(), hidden: false });
        assert_eq!(
            parse_args(args(&["--hidden", "--profile", "left"])).unwrap(),
            Launch { profile: "left".into(), hidden: true }
        );
        for bad in [&["--profile"][..], &["--profile", ""], &["--profile", "a b"], &["--profile", "../x"]] {
            assert!(parse_args(args(bad)).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn the_login_entry_starts_hidden_on_the_same_profile() {
        for profile in ["default", "left"] {
            let launch = parse_args(login::login_args(profile)).unwrap();
            assert_eq!(launch, Launch { profile: profile.into(), hidden: true });
        }
        assert_ne!(login::login_entry_name("default"), login::login_entry_name("left"));
    }
}
