// Preview data. Replaced by the daemon client once the Tauri IPC layer exists.
import { defaultDeviceName, os } from "@/lib/platform";
import type { Clip, Device, OpRow, Transfer, Workspace } from "./types";

const now = Date.now();
const m = (n: number) => now - n * 60_000;

export const ME = "d-studio";

export const devices: Device[] = [
  { id: ME, name: defaultDeviceName[os], platform: os, connection: "lan", latencyMs: 0, lastSeen: now, isMe: true, seq: 214, fingerprint: "7f3a·c91e·02bd·58a4" },
  { id: "d-mbp", name: os === "macos" ? "Mac mini" : "MacBook Pro", platform: "macos", connection: "lan", latencyMs: 1.6, lastSeen: now, isMe: false, seq: 188, fingerprint: "b40d·19ee·7a63·c0f2" },
  { id: "d-nuc", name: "Linux box", platform: "linux", connection: "direct", latencyMs: 38, lastSeen: m(3), isMe: false, seq: 96, fingerprint: "2c88·e5f1·4d07·93ab" },
  { id: "d-lap", name: "Old ThinkPad", platform: "windows", connection: "offline", latencyMs: null, lastSeen: m(60 * 26), isMe: false, seq: 41, fingerprint: "91ce·3b72·ad10·66e8" },
];

export const clips: Clip[] = [
  { id: "c1", device: "d-mbp", ts: m(1), kind: "link", text: "https://github.com/aurae-rs/aurae/pull/412", pinned: false, sensitive: false },
  { id: "c2", device: ME, ts: m(4), kind: "code", text: "fn order_key(&self) -> (Timestamp, DeviceId) {\n    (self.ts, self.device)\n}", pinned: true, sensitive: false },
  { id: "c3", device: "d-nuc", ts: m(11), kind: "text", text: "Sync after 1 week offline (10k ops) should land under 3 s. Measure on the real laptops, not loopback.", pinned: false, sensitive: false },
  { id: "c4", device: "d-mbp", ts: m(26), kind: "text", text: "Flight AC 837, seat 14A, confirmation K9Q2LM", pinned: true, sensitive: false },
  { id: "c5", device: ME, ts: m(47), kind: "code", text: "cargo run --release -p tether-sim -- --seeds 10000", pinned: false, sensitive: false },
  { id: "c6", device: ME, ts: m(62), kind: "text", text: "hunter2-correct-horse-battery", pinned: false, sensitive: true },
  { id: "c7", device: "d-mbp", ts: m(95), kind: "link", text: "https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/", pinned: false, sensitive: false },
  { id: "c8", device: "d-nuc", ts: m(180), kind: "text", text: "Meeting notes: move relay work to stage 3, ship LAN clipboard first.", pinned: false, sensitive: false },
  { id: "c9", device: ME, ts: m(60 * 29), kind: "code", text: "SELECT device, MAX(seq) FROM ops GROUP BY device;", pinned: false, sensitive: false },
];

export const transfers: Transfer[] = [
  { id: "t1", name: "demo-recording.mov", size: 2_400_000_000, from: ME, to: "d-mbp", bytesDone: 1_560_000_000, speed: 112_000_000, state: "active", resumedFrom: 900_000_000, updated: now },
  { id: "t2", name: "dataset-v3.tar.zst", size: 640_000_000, from: "d-nuc", to: ME, bytesDone: 220_000_000, speed: 0, state: "paused", resumedFrom: 0, updated: m(8) },
  { id: "t3", name: "Q4-report.pdf", size: 42_000_000, from: "d-mbp", to: ME, bytesDone: 42_000_000, speed: 0, state: "done", resumedFrom: 0, updated: m(34) },
];

export const workspaces: Workspace[] = [
  {
    id: "w1",
    name: "Tether · simulator work",
    device: ME,
    ts: m(52),
    tabs: [
      { title: "tether-sim/src/lib.rs", url: "github.com/you/tether" },
      { title: "FoundationDB: simulation testing", url: "apple.github.io/foundationdb" },
      { title: "CRDTs: the hard parts", url: "youtube.com" },
      { title: "docs.rs · quinn", url: "docs.rs/quinn" },
    ],
    files: ["crates/tether-core/src/log.rs", "crates/tether-sim/src/lib.rs", "docs/DESIGN.md"],
    terminals: ["~/Tether", "~/Tether/crates/tether-sim"],
  },
  {
    id: "w2",
    name: "Portfolio refresh",
    device: "d-mbp",
    ts: m(60 * 20),
    tabs: [
      { title: "abdkhan.com", url: "abdkhan.com" },
      { title: "Tailwind v4 docs", url: "tailwindcss.com" },
    ],
    files: ["src/index.css", "src/sections/Navbar/Navbar.jsx"],
    terminals: ["~/abdkhan-portfolio"],
  },
  {
    id: "w3",
    name: "Reading list",
    device: "d-nuc",
    ts: m(60 * 52),
    tabs: [
      { title: "The Part-Time Parliament", url: "lamport.azurewebsites.net" },
      { title: "TigerBeetle: a deterministic database", url: "tigerbeetle.com" },
      { title: "Hybrid logical clocks", url: "cse.buffalo.edu" },
    ],
    files: [],
    terminals: [],
  },
];

const kinds = ["clipboard.text", "clipboard.text", "clipboard.text", "pin", "settings.name"];
export const ops: OpRow[] = Array.from({ length: 14 }, (_, i) => {
  const dev = devices[i % 3];
  const seq = dev.seq - Math.floor(i / 3);
  const kind = kinds[i % kinds.length];
  return {
    device: dev.id,
    seq,
    hlc: `${(now - i * 41_000).toString(16).slice(-9)}.${(i * 7) % 16}`,
    kind,
    summary: kind === "pin" ? "pinned an item" : kind === "settings.name" ? "renamed a device" : "copied text",
    ts: now - i * 41_000,
  };
});

export const remoteCopies: { device: string; kind: Clip["kind"]; text: string }[] = [
  { device: "d-mbp", kind: "text", text: "Remember: mark password-manager clips as sensitive so they never sync." },
  { device: "d-nuc", kind: "link", text: "https://docs.rs/quinn/latest/quinn/struct.Endpoint.html" },
  { device: "d-mbp", kind: "code", text: "git log --oneline --graph --all -20" },
  { device: "d-nuc", kind: "text", text: "Measure p99 clipboard latency on two real machines, not loopback." },
];
