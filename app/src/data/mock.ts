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

/** base64url, so the sample JWT is built rather than pasted. */
const b64url = (o: object) => btoa(JSON.stringify(o)).replace(/=+$/, "").replace(/\+/g, "-").replace(/\//g, "_");
const nowS = Math.floor(now / 1000);
const sampleJwt = `${b64url({ alg: "HS256", typ: "JWT" })}.${b64url({ sub: "device:7f3a", scope: "sync:read sync:write", iat: nowS - 3600, exp: nowS + 6 * 3600 })}.dGV0aGVyLXByZXZpZXctc2lnbmF0dXJl`;

const clip = (id: string, device: string, minutesAgo: number, text: string, pinned = false): Clip => ({ id, device, ts: m(minutesAgo), text, pinned, sensitive: false });

/** A spread of what people really copy, so every kind of preview has an example. */
export const clips: Clip[] = [
  clip("c1", "d-mbp", 1, "https://github.com/tokio-rs/tokio/pull/6812?utm_source=newsletter&utm_medium=email&utm_campaign=rust-weekly"),
  clip("c2", ME, 3, `{"device":{"id":"7f3a-c91e","name":"Windows PC","platform":"windows"},"sync":{"clipboard":true,"files":true,"maxHistory":1000,"neverSync":["password","token"]},"peers":[{"id":"b40d","transport":"quic","lan":true},{"id":"2c88","transport":"quic","lan":false}],"version":3}`),
  clip("c3", ME, 4, "fn order_key(&self) -> (Timestamp, DeviceId) {\n        (self.ts, self.device)\n    }", true),
  clip("c4", "d-nuc", 9, "select device, max(seq) as head, count(*) as ops from oplog where ts > now() - interval '1 day' group by device order by head desc;"),
  clip("c5", "d-mbp", 14, "#30D158"),
  clip("c6", ME, 20, "1767225600"),
  clip("c7", "d-mbp", 26, "Flight AC 837, seat 14A, confirmation K9Q2LM", true),
  clip("c8", ME, 33, "def backoff(attempt: int, base: float = 0.25) -> float:\n\t\tdelay = base * (2 ** attempt)\n\t\treturn min(delay, 30.0) * random.uniform(0.5, 1.5)"),
  clip("c9", "d-nuc", 47, "task,stage,status\nLAN clipboard,1,Done\nPairing handshake,1,In progress\nReplicated log,2,Done\nResumable transfers,3,Planned"),
  clip("c10", ME, 62, "AKIAIOSFODNN7EXAMPLE"),
  clip("c11", "d-mbp", 80, "Tether keeps an append-only log on every device and\nreplicates it to each paired peer, so that copies made\nwhile offline still converge once the devices\nreconnect, without a server holding the data."),
  clip("c12", ME, 95, "C:\\Users\\you\\Documents\\Tether\\design-review.pdf"),
  clip("c13", "d-nuc", 120, "(1920 * 1080 * 4) / 1024 / 1024"),
  clip("c14", "d-mbp", 150, sampleJwt),
  clip("c15", ME, 180, "export const backoff=(attempt:number,base=250)=>Math.min(base*2**attempt,30_000)"),
  clip("c16", "d-nuc", 240, "relay-ops@example.com"),
  clip("c17", ME, 60 * 29, "Xk9#mQ2$vL7pR4"),
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

export const remoteCopies: { device: string; text: string }[] = [
  { device: "d-mbp", text: "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s&si=Ab3dEfGhIjKlMn" },
  { device: "d-nuc", text: "Get-ChildItem -Path C:\\Projects -Recurse -Filter *.log | Remove-Item -WhatIf" },
  { device: "d-mbp", text: `[{"seq":215,"device":"b40d","kind":"clipboard.text"},{"seq":216,"device":"b40d","kind":"pin"}]` },
  { device: "d-nuc", text: "git log --oneline --graph --all -20" },
  { device: "d-mbp", text: "hsl(212, 92%, 58%)" },
];
