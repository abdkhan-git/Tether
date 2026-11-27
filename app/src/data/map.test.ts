import { describe, expect, it } from "vitest";
import type { DaemonClip, DaemonDevice } from "./daemon";
import { applyItemChange, clipOf, devicesOf, fingerprint, itemOf, mergeClips, nearbyOf, opsOf, withArticle } from "./map";

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
  return { author, seq, at, counter, text, held: null, pinned: false };
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

  it("reads each device's health, its OS from its heartbeats, and whether it can be woken", () => {
    const health = { cpu: 12.5, memUsed: 4, memTotal: 16, diskFree: 100, diskTotal: 500, battery: null, uptimeS: 60, os: "Windows 11 Pro", version: "0.1.0" };
    const withPulse: DaemonDevice[] = [
      { ...list[0], pulse: { health, heardMsAgo: 400, liveness: "suspect", wakeable: true } },
      { ...list[1], pulse: { health: { ...health, os: "macOS 15.2" }, heardMsAgo: 0, liveness: null, wakeable: false } },
      { id: gone, name: "Asleep", state: "offline", rttMs: null, ops: 4, pulse: { health: null, heardMsAgo: 3_600_000, liveness: null, wakeable: true } },
    ];
    const [mine, laptop, asleep] = devicesOf(withPulse, "windows", [], 10_000_000);
    expect(mine.platform).toBe("windows");
    expect(laptop).toMatchObject({ platform: "windows", liveness: "suspect", wakeable: false, health: { cpu: 12.5 } });
    expect(asleep).toMatchObject({ connection: "offline", lastSeen: 6_400_000, wakeable: true, liveness: null, platform: null });
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

  it("applies pins and deletes reported by the daemon", () => {
    const clips = mergeClips([], [item(me, 1, 100), item(peer, 1, 200)].map((i) => clipOf(i)));
    const pinned = applyItemChange(clips, { author: me, seq: 1, pinned: true, deleted: false });
    expect(pinned.find((c) => c.id === `${me}:1`)?.pinned).toBe(true);
    const deleted = applyItemChange(pinned, { author: peer, seq: 1, pinned: false, deleted: true });
    expect(deleted.map((c) => c.id)).toEqual([`${me}:1`]);
    expect(clipOf({ ...item(me, 3, 1), pinned: true }).pinned).toBe(true);
  });

  it("parses clip ids back into daemon item names, and nothing else", () => {
    expect(itemOf(`${me}:12`)).toEqual({ author: me, seq: 12 });
    for (const bad of ["c1", `${me}:0`, `${me}:x`, `zz:1`, `${me}:1.5`]) expect(itemOf(bad)).toBeNull();
  });

  it("picks the right article for a kind of secret", () => {
    expect(withArticle("AWS access key")).toBe("an AWS access key");
    expect(withArticle("GitHub token")).toBe("a GitHub token");
  });
});
