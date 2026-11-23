// UI-side shapes. They mirror what the daemon will expose over Tauri IPC:
// `Op` in tether-core is (device, seq, ts, kind), so OpRow carries the same fields.

export type Platform = "windows" | "macos" | "linux";
export type Connection = "lan" | "direct" | "relay" | "offline";

export interface Device {
  id: string;
  name: string;
  platform: Platform;
  connection: Connection;
  latencyMs: number | null;
  lastSeen: number;
  isMe: boolean;
  /** Highest seq held for this device's own log (version-vector entry). */
  seq: number;
  fingerprint: string;
}

export type ClipKind = "text" | "link" | "code";

export interface Clip {
  id: string;
  device: string;
  ts: number;
  kind: ClipKind;
  text: string;
  pinned: boolean;
  /** Sensitive items never leave the device that copied them. */
  sensitive: boolean;
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
