// Turns what the daemon reports into what the UI shows. Pure functions, so
// they are unit-tested (map.test.ts) without a daemon.

import { contentOf } from "@/lib/content/detect";
import type { OS } from "@/lib/platform";
import type { DaemonClip, DaemonDevice } from "./daemon";
import { newestFirst, type Clip, type Device, type OpRow } from "./types";

/** The start of a device key in four groups of four, for comparing by eye or aloud. */
export function fingerprint(id: string): string {
  return [0, 4, 8, 12].map((i) => id.slice(i, i + 4)).join("·");
}

/** A short stand-in when a device has not told us its name. */
export function shortId(id: string): string {
  return id.slice(0, 12);
}

/**
 * Members of the group, this device first. Revoked devices and unpaired
 * devices nearby are left out (`nearbyOf` lists the latter).
 */
export function devicesOf(list: DaemonDevice[], thisOs: OS, previous: Device[], now = Date.now()): Device[] {
  const before = new Map(previous.map((d) => [d.id, d]));
  return list
    .filter((d) => d.state === "thisDevice" || d.state === "connected" || d.state === "offline")
    .map((d) => {
      const isMe = d.state === "thisDevice";
      const connected = isMe || d.state === "connected";
      return {
        id: d.id,
        name: d.name.trim() || shortId(d.id),
        platform: isMe ? thisOs : null,
        connection: connected ? "lan" : "offline",
        latencyMs: isMe ? 0 : connected ? d.rttMs : null,
        lastSeen: connected ? now : (before.get(d.id)?.lastSeen ?? null),
        isMe,
        seq: d.ops,
        fingerprint: fingerprint(d.id),
      } satisfies Device;
    })
    .sort((a, b) => Number(b.isMe) - Number(a.isMe));
}

export function nearbyOf(list: DaemonDevice[]): { id: string; name: string }[] {
  return list.filter((d) => d.state === "nearby").map((d) => ({ id: d.id, name: d.name.trim() || shortId(d.id) }));
}

export function clipOf(item: DaemonClip, fresh = false): Clip {
  return {
    id: `${item.author}:${item.seq}`,
    device: item.author,
    ts: item.at,
    counter: item.counter,
    text: item.text,
    pinned: false,
    // The daemon never syncs secrets; this masks things like access tokens on screen.
    sensitive: contentOf(item.text).sensitive,
    held: item.held,
    fresh,
  };
}

/** Add `incoming` to `clips` (replacing any copy with the same id), newest first. */
export function mergeClips(clips: Clip[], incoming: Clip[]): Clip[] {
  const ids = new Set(incoming.map((c) => c.id));
  return [...incoming, ...clips.filter((c) => !ids.has(c.id))].sort(newestFirst);
}

/** The diagnostics log view, from what the daemon exposes: its clipboard operations. */
export function opsOf(clips: Clip[], limit = 40): OpRow[] {
  return clips.slice(0, limit).map((c) => ({
    device: c.device,
    seq: Number(c.id.split(":")[1] ?? 0),
    hlc: `${c.ts.toString(16).slice(-9)}.${c.counter ?? 0}`,
    kind: "clipboard.text",
    summary: "copied text",
    ts: c.ts,
  }));
}

const holdText = {
  hiddenCharacters: "it hides control characters",
  dangerousCommand: "it looks like a command that downloads, runs or deletes things",
  multiLineCommand: "it is a multi-line shell command",
} as const;

/** Why a held copy was kept off the clipboard, as a sentence ending. */
export function heldBecause(reason: keyof typeof holdText): string {
  return holdText[reason];
}

/** "an AWS access key", "a GitHub token": the daemon names the kind of secret. */
export function withArticle(kind: string): string {
  return /^[aeiou]/i.test(kind) ? `an ${kind}` : `a ${kind}`;
}
