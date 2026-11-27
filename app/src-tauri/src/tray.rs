//! The tray (Windows notification area, macOS menu bar, Linux status area):
//! open the window, pause sync, quit. Its state comes from the daemon, never
//! from the menu's own check mark, so the tray, the window and the CLI
//! always agree.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tether_ipc::{Reply, Request};
use tracing::warn;

use crate::link::Link;

pub struct Tray {
    icon: TrayIcon<Wry>,
    pause: CheckMenuItem<Wry>,
}

pub fn build(app: &AppHandle) -> tauri::Result<Tray> {
    let open = MenuItem::with_id(app, "open", "Open Tether", true, None::<&str>)?;
    let scratchpad = MenuItem::with_id(app, "scratchpad", "Scratchpad", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pause sync", false, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Tether", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &scratchpad,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Tether")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_window(app),
            "scratchpad" => crate::open_scratchpad(app),
            "pause" => toggle_pause(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    Ok(Tray { icon: builder.build(app)?, pause })
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_pause(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let link = app.state::<Arc<Link>>().inner().clone();
        let result = match link.call(Request::Status, true).await {
            Ok(Reply::Status(s)) => link.call(Request::SetPaused { paused: !s.paused }, true).await.map(drop),
            Ok(other) => Err(format!("unexpected reply: {other:?}")),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            warn!(error = %e, "could not change the pause setting");
        }
        // The click toggled the check mark by itself; put back the daemon's truth.
        refresh(&app);
    });
}

/// Ask the daemon for its status and show it: the pause check mark, and a
/// tooltip saying how many devices are connected or why sync is off.
pub fn refresh(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let link = app.state::<Arc<Link>>().inner().clone();
        let status = match link.call(Request::Status, true).await {
            Ok(Reply::Status(s)) => Some(s),
            _ => None,
        };
        let Some(tray) = app.try_state::<Tray>() else { return };
        let (tooltip, paused, enabled) = match &status {
            None => ("Tether: not running".to_string(), false, false),
            Some(s) if s.revoked => ("Tether: removed from its group".to_string(), s.paused, false),
            Some(s) if s.paused => ("Tether: sync paused".to_string(), true, true),
            Some(s) => {
                let others = s.members.saturating_sub(1);
                let text = if others == 0 {
                    "Tether: no devices paired yet".to_string()
                } else {
                    format!("Tether: {} of {} devices connected", s.connected, others)
                };
                (text, false, true)
            }
        };
        let _ = tray.icon.set_tooltip(Some(tooltip));
        let _ = tray.pause.set_checked(paused);
        let _ = tray.pause.set_enabled(enabled);
    });
}
