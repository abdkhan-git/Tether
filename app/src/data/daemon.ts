// The desktop app's bridge to tetherd. Each function is one Tauri command,
// granted to this window in src-tauri/capabilities/main.json; the Rust side
// checks every argument and forwards it over the local control channel.
// Shapes mirror src-tauri/src/view.rs.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { HoldReason, LinkState, PairCode } from "./types";

/** True inside the Tauri window; false in the browser preview (`npm run dev`). */
export const inDesktopApp = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export interface DaemonStatus {
  device: string;
  name: string;
  root: string;
  members: number;
  connected: number;
  revoked: boolean;
  port: number;
  pairingOpen: boolean;
  paused: boolean;
}

export type DeviceState = "thisDevice" | "connected" | "offline" | "revoked" | "nearby";

export interface DaemonDevice {
  id: string;
  /** Chosen by the device itself: display only. */
  name: string;
  state: DeviceState;
  rttMs: number | null;
  ops: number;
}

export interface Cursor {
  at: number;
  counter: number;
  device: string;
}

export interface DaemonClip {
  author: string;
  seq: number;
  at: number;
  counter: number;
  text: string;
  held: HoldReason | null;
  pinned: boolean;
}

export interface HistoryPage {
  items: DaemonClip[];
  next: Cursor | null;
}

export type DaemonEvent =
  | { type: "clipboard"; item: DaemonClip }
  | { type: "historyChanged" }
  | { type: "keptLocal"; kind: string }
  | { type: "devicesChanged" }
  | { type: "paused"; paused: boolean }
  | { type: "itemChanged"; author: string; seq: number; pinned: boolean; deleted: boolean }
  | { type: "lagged" };

export const daemon = {
  status: () => invoke<DaemonStatus>("status"),
  devices: () => invoke<DaemonDevice[]>("devices"),
  history: (before: Cursor | null, limit: number) => invoke<HistoryPage>("history", { before, limit }),
  pair: () => invoke<PairCode>("pair"),
  join: (code: string, addr: string | null) => invoke<string>("join", { code, addr }),
  revoke: (device: string) => invoke<void>("revoke", { device }),
  setPaused: (paused: boolean) => invoke<boolean>("set_paused", { paused }),
  setPinned: (author: string, seq: number, pinned: boolean) => invoke<void>("set_pinned", { author, seq, pinned }),
  deleteItem: (author: string, seq: number) => invoke<void>("delete_item", { author, seq }),
  openLink: (url: string) => invoke<void>("open_link", { url }),
  autostart: () => invoke<boolean>("autostart"),
  setAutostart: (enabled: boolean) => invoke<boolean>("set_autostart", { enabled }),
  onEvent: (f: (e: DaemonEvent) => void): Promise<UnlistenFn> => listen<DaemonEvent>("tether://event", (e) => f(e.payload)),
  onLink: (f: (s: LinkState) => void): Promise<UnlistenFn> => listen<LinkState>("tether://link", (e) => f(e.payload)),
};

/** Tauri rejects with the command's error string; anything else, with an Error. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Something went wrong";
}
