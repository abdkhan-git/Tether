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
mod tray;
mod view;

use std::sync::Arc;

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tether_ipc::{Reply, Request};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::link::Link;

const DEFAULT_PROFILE: &str = "default";
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

// --- Startup ---

/// `--profile NAME` picks the daemon to talk to (as for tetherd and the CLI).
fn profile_from_args() -> Result<String, String> {
    let mut args = std::env::args().skip(1);
    let mut profile = DEFAULT_PROFILE.to_string();
    while let Some(arg) = args.next() {
        if arg == "--profile" {
            profile = args.next().ok_or("--profile needs a name")?;
        }
    }
    let valid = !profile.is_empty()
        && profile.len() <= 32
        && profile.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if valid {
        Ok(profile)
    } else {
        Err("--profile may only contain letters, digits, '-' and '_' (at most 32)".into())
    }
}

fn main_window(app: &AppHandle) -> tauri::Result<()> {
    let dev = tauri::is_dev();
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Tether")
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
    let profile = match profile_from_args() {
        Ok(p) => p,
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
        .manage(link.clone())
        .invoke_handler(tauri::generate_handler![status, devices, history, pair, join, revoke, set_paused, open_link])
        .setup(move |app| {
            let handle = app.handle().clone();
            main_window(&handle)?;
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
