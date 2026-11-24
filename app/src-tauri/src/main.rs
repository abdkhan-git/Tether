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
mod notify;
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
/// Tells the palette window to start a fresh search.
const PALETTE_OPEN: &str = "palette://open";
/// The quick paste shortcut. Not Ctrl+Shift+V: browsers, Office and editors
/// use that for "paste as plain text", and a global shortcut would take it.
const PALETTE_SHORTCUT: &str = "Alt+Shift+V";

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

/// Put a history item on this device's clipboard (through the daemon, so it
/// is not recorded again as a new copy).
#[tauri::command]
async fn copy_here(link: State<'_, Arc<Link>>, author: String, seq: u64) -> Res<()> {
    match link.call(Request::CopyHere { author: view::device_id(&author)?, seq }, true).await? {
        Reply::Copied => Ok(()),
        r => Err(unexpected(r)),
    }
}

/// The quick paste shortcut, if it could be registered (another app may hold it).
struct PaletteShortcut(std::sync::Mutex<Option<String>>);

#[tauri::command]
fn palette_shortcut(shortcut: State<'_, PaletteShortcut>) -> Option<String> {
    shortcut.0.lock().ok().and_then(|s| s.clone())
}

#[tauri::command]
async fn send_item(link: State<'_, Arc<Link>>, author: String, seq: u64, to: String) -> Res<()> {
    let req = Request::Send { author: view::device_id(&author)?, seq, to: view::device_id(&to)? };
    match link.call(req, false).await? {
        Reply::Sent => Ok(()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
async fn rename_device(link: State<'_, Arc<Link>>, device: String, name: String) -> Res<()> {
    let req = Request::Rename { device: view::device_id(&device)?, name: view::device_name(&name)? };
    match link.call(req, true).await? {
        Reply::Renamed => Ok(()),
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

/// The app's own settings, and the notification throttle.
struct AppSettings {
    profile: String,
    settings: std::sync::Mutex<notify::Settings>,
    throttle: std::sync::Mutex<notify::Throttle>,
}

/// Whether a copy from another device shows a notification.
#[tauri::command]
fn notifications(app: State<'_, AppSettings>) -> bool {
    app.settings.lock().map(|s| s.notifications).unwrap_or(true)
}

#[tauri::command]
fn set_notifications(app: State<'_, AppSettings>, enabled: bool) -> Res<bool> {
    let settings = {
        let mut s = app.settings.lock().map_err(|_| "settings unavailable".to_string())?;
        s.notifications = enabled;
        *s
    };
    notify::save(&app.profile, &settings)?;
    Ok(enabled)
}

/// What a notification is about.
enum Notice {
    /// A copy arrived from another device.
    Copy(tether_ipc::HistoryItem),
    /// Another device sent this one an item.
    Send { from: tether_core::DeviceId, held: bool },
}

/// Show a notification for `notice` if `notify` says so.
fn notify_event(app: &AppHandle, notice: Notice) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = app.state::<AppSettings>();
        let enabled = settings.settings.lock().map(|s| s.notifications).unwrap_or(false);
        if !enabled {
            return;
        }
        let link = app.state::<Arc<Link>>().inner().clone();
        let Ok(Reply::Status(status)) = link.call(Request::Status, true).await else { return };
        let sender = match &notice {
            Notice::Copy(item) => item.author,
            Notice::Send { from, .. } => *from,
        };
        let from = match link.call(Request::Devices, true).await {
            Ok(Reply::Devices(list)) => list.into_iter().find(|d| d.id == sender).map(|d| d.name),
            _ => None,
        }
        .unwrap_or_default();
        let window_focused = app
            .get_webview_window("main")
            .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
        let cx = notify::Context { enabled, paused: status.paused, window_focused, me: status.device, from: &from };
        let shown = match &notice {
            Notice::Copy(item) => notify::for_copy(item, &cx),
            Notice::Send { from, held } => notify::for_send(*from, *held, &cx),
        };
        let Some((title, body)) = shown else { return };
        if !settings.throttle.lock().map(|mut t| t.allow(std::time::Instant::now())).unwrap_or(false) {
            return;
        }
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            warn!(error = %e, "could not show a notification");
        }
    });
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

/// The rules every Tether window follows: it shows only the bundled page,
/// opens no new windows, and has no native file drop.
fn guarded<'a>(
    builder: WebviewWindowBuilder<'a, tauri::Wry, AppHandle>,
) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    let dev = tauri::is_dev();
    builder
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
}

/// The quick paste palette: a small window over whatever app is in front,
/// hidden until its shortcut is pressed, hidden again when it loses focus.
fn palette_window(app: &AppHandle) -> tauri::Result<()> {
    let window = guarded(WebviewWindowBuilder::new(app, "palette", WebviewUrl::App("index.html".into())))
        .title("Tether quick paste")
        .visible(false)
        .focused(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .inner_size(640.0, 440.0)
        .center()
        .build()?;
    let hide = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            let _ = hide.hide();
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = hide.hide();
        }
        _ => {}
    });
    Ok(())
}

/// Register the quick paste shortcut. If another app holds it, the app runs
/// on without it and the page says so (palette_shortcut returns None).
fn register_palette_shortcut(app: &AppHandle) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let result = app.global_shortcut().on_shortcut(PALETTE_SHORTCUT, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            open_palette(app);
        }
    });
    match result {
        Ok(()) => {
            if let Ok(mut slot) = app.state::<PaletteShortcut>().0.lock() {
                *slot = Some(PALETTE_SHORTCUT.to_string());
            }
            info!(shortcut = PALETTE_SHORTCUT, "quick paste shortcut registered");
        }
        Err(e) => warn!(shortcut = PALETTE_SHORTCUT, error = %e, "could not register the quick paste shortcut"),
    }
}

/// Show the palette in front, centred, and tell it to start a fresh search.
fn open_palette(app: &AppHandle) {
    if let Some(palette) = app.get_webview_window("palette") {
        let _ = palette.center();
        let _ = palette.show();
        let _ = palette.set_focus();
        let _ = palette.emit_to("palette", PALETTE_OPEN, ());
    }
}

fn main_window(app: &AppHandle, hidden: bool) -> tauri::Result<()> {
    let window = guarded(WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into())))
        .title("Tether")
        .visible(!hidden)
        // A start at sign-in stays in the tray: no focus taken from what the user is doing.
        .focused(!hidden)
        .inner_size(1240.0, 820.0)
        .min_inner_size(900.0, 600.0)
        .center()
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
        // Used from Rust only (notify_event): its commands are not granted to the page.
        .plugin(tauri_plugin_notification::init())
        // The palette shortcut, registered in `setup`. Rust side only.
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(PaletteShortcut(std::sync::Mutex::new(None)))
        .manage(LoginEntry(login::entry(&profile)))
        .manage(AppSettings {
            profile: profile.clone(),
            settings: std::sync::Mutex::new(notify::load(&profile)),
            throttle: std::sync::Mutex::new(notify::Throttle::default()),
        })
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
            rename_device,
            send_item,
            delete_item,
            open_link,
            autostart,
            set_autostart,
            notifications,
            set_notifications,
            copy_here,
            palette_shortcut
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            main_window(&handle, hidden)?;
            palette_window(&handle)?;
            register_palette_shortcut(&handle);
            let tray = tray::build(&handle)?;
            app.manage(tray);
            info!(%profile, "Tether started");

            let (events_to, state_to) = (handle.clone(), handle.clone());
            tauri::async_runtime::spawn(events_link.run_events(
                move |event| {
                    if matches!(event, tether_ipc::Event::Paused(_) | tether_ipc::Event::DevicesChanged) {
                        tray::refresh(&events_to);
                    }
                    if let tether_ipc::Event::Received { from, held, .. } = &event {
                        notify_event(&events_to, Notice::Send { from: *from, held: *held });
                    }
                    if let tether_ipc::Event::Clipboard(item) = &event {
                        notify_event(&events_to, Notice::Copy(item.clone()));
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
