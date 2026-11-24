//! Tauri's build step. Listing the app's commands here makes Tauri generate an
//! `allow-<command>` permission for each, and refuse any command a window's
//! capability does not grant (capabilities/main.json). SECURITY.md rule 10.

/// Every command the page may call. Keep in step with `generate_handler!` in main.rs.
const COMMANDS: &[&str] = &[
    "status",
    "devices",
    "history",
    "pair",
    "join",
    "revoke",
    "set_paused",
    "set_pinned",
    "rename_device",
    "send_item",
    "copy_here",
    "palette_shortcut",
    "delete_item",
    "open_link",
    "autostart",
    "set_autostart",
    "notifications",
    "set_notifications",
];

fn main() {
    let attributes = tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("running tauri-build");
}
