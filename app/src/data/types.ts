// UI-side shapes. Inside the desktop app they are filled from the daemon
// (`live.ts` maps the control channel's types onto these); in the browser
// preview, from `mock.ts`. `Op` in tether-core is (device, seq, ts, kind), so
// OpRow carries the same fields.

export type Platform = "windows" | "macos" | "linux";
export type Connection = "lan" | "direct" | "relay" | "offline";

export interface Device {
  id: string;
  name: string;
  /** Known for this device. Other devices do not report their OS yet, so it is null for them. */
  platform: Platform | null;
  connection: Connection;
  /** Measured QUIC round trip, while connected. */
  latencyMs: number | null;
  /** When this device was last seen connected, if it has been during this session. */
  lastSeen: number | null;
  isMe: boolean;
  /** Highest seq held for this device's own log (version-vector entry). */
  seq: number;
  /** The start of the device's public key, grouped for reading aloud. */
  fingerprint: string;
}

/** Why a copy from another device was kept off the system clipboard. */
export type HoldReason = "hiddenCharacters" | "dangerousCommand" | "multiLineCommand";

export interface Clip {
  id: string;
  device: string;
  ts: number;
  /** Hybrid logical clock counter: orders copies made in the same millisecond. */
  counter?: number;
  text: string;
  pinned: boolean;
  /** Sensitive items never leave the device that copied them. */
  sensitive: boolean;
  /** Set when the daemon held this copy in history instead of applying it. */
  held?: HoldReason | null;
  fresh?: boolean;
}

export type TransferState = "active" | "paused" | "done";

export interface Transfer {
  id: string;
  name: string;
  size: number;
  from: string;
  to: string;
  bytesDone: number;
  speed: number;
  state: TransferState;
  /** Bytes already on the receiver before this attempt (resume). */
  resumedFrom: number;
  updated: number;
}

export interface Workspace {
  id: string;
  name: string;
  device: string;
  ts: number;
  tabs: { title: string; url: string }[];
  files: string[];
  terminals: string[];
}

export interface OpRow {
  device: string;
  seq: number;
  hlc: string;
  kind: string;
  summary: string;
  ts: number;
}

export interface Toast {
  id: number;
  message: string;
  description?: string;
}

/** Whether the app can reach its daemon. Always "connected" in the browser preview. */
export type LinkState = { state: "connecting" } | { state: "connected" } | { state: "unreachable"; reason: string };

export interface PairCode {
  code: string;
  validSecs: number;
  attempts: number;
}

/** Newest first, in the daemon's order: time, then counter, then device id. */
export function newestFirst(a: Clip, b: Clip): number {
  return b.ts - a.ts || (b.counter ?? 0) - (a.counter ?? 0) || (a.device < b.device ? 1 : a.device > b.device ? -1 : 0);
}
