// What device is this UI running on? Shortcuts, file-manager names and the
// "this device" label all follow the answer, so the app speaks Windows on
// Windows and macOS on a Mac. Inside Tauri the OS plugin can replace this.

export type OS = "windows" | "macos" | "linux";

function detect(): OS {
  if (typeof navigator === "undefined") return "windows";
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const raw = `${nav.userAgentData?.platform ?? ""} ${navigator.platform ?? ""} ${navigator.userAgent ?? ""}`.toLowerCase();
  if (/mac|iphone|ipad/.test(raw)) return "macos";
  if (/win/.test(raw)) return "windows";
  return "linux";
}

export const os: OS = detect();
export const isMac = os === "macos";

/** The modifier key as this OS labels it. */
export const modKey = isMac ? "⌘" : "Ctrl";

/** Keyboard hint for the global search shortcut (focus the search field). */
export const searchShortcut = isMac ? "⌘F" : "Ctrl+F";

export const fileManager: Record<OS, string> = {
  windows: "File Explorer",
  macos: "Finder",
  linux: "Files",
};

export const osLabel: Record<OS, string> = {
  windows: "Windows",
  macos: "macOS",
  linux: "Linux",
};

/** Name given to this machine until the user renames it. */
export const defaultDeviceName: Record<OS, string> = {
  windows: "Windows PC",
  macos: "MacBook",
  linux: "Linux PC",
};

/** True when the user is pressing the platform's primary modifier. */
export function isMod(e: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return isMac ? e.metaKey : e.ctrlKey;
}

/** "this PC", "this Mac": how copy actions refer to the local machine. */
export const thisDevice: Record<OS, string> = {
  windows: "this PC",
  macos: "this Mac",
  linux: "this computer",
};
