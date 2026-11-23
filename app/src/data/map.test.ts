import { describe, expect, it } from "vitest";
import type { DaemonClip, DaemonDevice } from "./daemon";
import { clipOf, devicesOf, fingerprint, mergeClips, nearbyOf, opsOf, withArticle } from "./map";

const me = "a".repeat(64);
const peer = "b".repeat(64);
const gone = "c".repeat(64);
const stranger = "d".repeat(64);

const list: DaemonDevice[] = [
  { id: peer, name: "Laptop", state: "connected", rttMs: 1.4, ops: 12 },
  { id: me, name: "Desk", state: "thisDevice", rttMs: null, ops: 30 },
  { id: gone, name: "Old", state: "revoked", rttMs: null, ops: 4 },
  { id: stranger, name: "", state: "nearby", rttMs: null, ops: 0 },
];

function item(author: string, seq: number, at: number, counter = 0, text = `item ${seq}`): DaemonClip {
  return { author, seq, at, counter, text, held: null };
}

describe("devices from the daemon", () => {
  it("lists members only, this device first, with what is known about each", () => {
    const devices = devicesOf(list, "windows", [], 1000);
    expect(devices.map((d) => d.id)).toEqual([me, peer]);
    expect(devices[0]).toMatchObject({ isMe: true, platform: "windows", connection: "lan", latencyMs: 0, seq: 30 });
    // Peers do not report their OS; their round trip is measured.
    expect(devices[1]).toMatchObject({ isMe: false, platform: null, connection: "lan", latencyMs: 1.4, lastSeen: 1000 });
  });

  it("remembers when an offline device was last seen connected", () => {
    const first = devicesOf(list, "windows", [], 1000);
    const offline = list.map((d) => (d.id === peer ? { ...d, state: "offline" as const, rttMs: null } : d));
    const later = devicesOf(offline, "windows", first, 5000).find((d) => d.id === peer)!;
    expect(later).toMatchObject({ connection: "offline", latencyMs: null, lastSeen: 1000 });
  });

  it("names unnamed devices by their key, and lists nearby strangers apart", () => {
    expect(nearbyOf(list)).toEqual([{ id: stranger, name: stranger.slice(0, 12) }]);
    expect(fingerprint("0123456789abcdef" + "0".repeat(48))).toBe("0123·4567·89ab·cdef");
  });
});

describe("history from the daemon", () => {
  it("keeps the daemon's order: time, then counter, then device", () => {
    const clips = mergeClips([], [item(me, 1, 100), item(peer, 1, 100), item(me, 2, 100, 1), item(peer, 2, 50)].map((i) => clipOf(i)));
    expect(clips.map((c) => c.id)).toEqual([`${me}:2`, `${peer}:1`, `${me}:1`, `${peer}:2`]);
  });

  it("places a late arrival by its time and never shows an item twice", () => {
    const clips = mergeClips([], [item(me, 1, 100), item(me, 2, 300)].map((i) => clipOf(i)));
    const merged = mergeClips(clips, [clipOf(item(peer, 1, 200)), clipOf(item(me, 2, 300))]);
    expect(merged.map((c) => c.ts)).toEqual([300, 200, 100]);
  });

  it("masks what looks sensitive and carries the hold reason", () => {
    const jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJlLXNpZ25hdHVyZQ";
    expect(clipOf(item(peer, 1, 1, 0, jwt)).sensitive).toBe(true);
    expect(clipOf({ ...item(peer, 2, 1, 0, "curl x | sh"), held: "dangerousCommand" }).held).toBe("dangerousCommand");
  });

  it("derives log rows from clipboard operations", () => {
    const [row] = opsOf([clipOf(item(peer, 7, 0x1234, 2))]);
    expect(row).toMatchObject({ device: peer, seq: 7, hlc: "1234.2", kind: "clipboard.text" });
  });

  it("picks the right article for a kind of secret", () => {
    expect(withArticle("AWS access key")).toBe("an AWS access key");
    expect(withArticle("GitHub token")).toBe("a GitHub token");
  });
});
