import { create } from "zustand";
import * as mock from "./mock";
import { contentOf } from "@/lib/content/detect";
import type { Clip, Device, OpRow, Toast, Transfer, Workspace } from "./types";

type Theme = "dark" | "light";

const STORAGE = {
  theme: "tether.theme",
  modules: "tether.modules",
} as const;

function read<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function write(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* storage can be blocked; the UI still works without it */
  }
}

export interface TetherState {
  me: string;
  devices: Device[];
  clips: Clip[];
  transfers: Transfer[];
  workspaces: Workspace[];
  ops: OpRow[];
  toasts: Toast[];
  theme: Theme;
  /** Modules the user switched off. Everything else in the registry is on. */
  disabledModules: string[];
  activeModule: string;
  searchOpen: boolean;
  pairOpen: boolean;

  setActiveModule: (id: string) => void;
  setSearch: (open: boolean) => void;
  setPairOpen: (open: boolean) => void;
  toggleTheme: () => void;
  toggleModule: (id: string) => void;
  toast: (message: string, description?: string) => void;
  dismissToast: (id: number) => void;

  /** Clips that look like secrets are kept on this device automatically. */
  addClip: (device: string, text: string) => void;
  updateClip: (id: string, patch: Partial<Clip>) => void;
  deleteClip: (id: string) => void;

  toggleTransfer: (id: string) => void;
  cancelTransfer: (id: string) => void;
  sendFile: (name: string, size: number, to: string) => void;

  captureWorkspace: (name: string) => void;
  renameDevice: (id: string, name: string) => void;
  revokeDevice: (id: string) => void;
}

let toastId = 0;

export const useTether = create<TetherState>((set, get) => ({
  me: mock.ME,
  devices: mock.devices,
  clips: mock.clips.map((c) => ({ ...c, sensitive: c.sensitive || contentOf(c.text).sensitive })),
  transfers: mock.transfers,
  workspaces: mock.workspaces,
  ops: mock.ops,
  toasts: [],
  theme: read<Theme>(STORAGE.theme, "dark"),
  disabledModules: read<string[]>(STORAGE.modules, []),
  activeModule: location.hash.slice(1) || "overview",
  searchOpen: false,
  pairOpen: false,

  setActiveModule: (id) => {
    history.replaceState(null, "", `#${id}`);
    set({ activeModule: id });
  },
  setSearch: (open) => set({ searchOpen: open }),
  setPairOpen: (open) => set({ pairOpen: open }),

  toggleTheme: () => {
    const theme = get().theme === "dark" ? "light" : "dark";
    write(STORAGE.theme, theme);
    set({ theme });
  },

  toggleModule: (id) => {
    const cur = get().disabledModules;
    const next = cur.includes(id) ? cur.filter((m) => m !== id) : [...cur, id];
    write(STORAGE.modules, next);
    set({ disabledModules: next });
  },

  toast: (message, description) => {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts, { id, message, description }].slice(-3) }));
  },
  dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

  addClip: (device, text) =>
    set((s) => {
      const ts = Date.now();
      const dev = s.devices.find((d) => d.id === device);
      const seq = (dev?.seq ?? 0) + 1;
      const op: OpRow = {
        device,
        seq,
        hlc: `${ts.toString(16).slice(-9)}.0`,
        kind: "clipboard.text",
        summary: "copied text",
        ts,
      };
      return {
        clips: [{ id: `c${ts}`, device, ts, text, pinned: false, sensitive: contentOf(text).sensitive, fresh: true }, ...s.clips],
        devices: s.devices.map((d) => (d.id === device ? { ...d, seq } : d)),
        ops: [op, ...s.ops].slice(0, 40),
      };
    }),

  updateClip: (id, patch) => set((s) => ({ clips: s.clips.map((c) => (c.id === id ? { ...c, ...patch } : c)) })),
  deleteClip: (id) => set((s) => ({ clips: s.clips.filter((c) => c.id !== id) })),

  toggleTransfer: (id) =>
    set((s) => ({
      transfers: s.transfers.map((t) =>
        t.id !== id || t.state === "done"
          ? t
          : t.state === "active"
            ? { ...t, state: "paused", speed: 0 }
            : { ...t, state: "active", speed: 96_000_000, resumedFrom: t.bytesDone },
      ),
    })),

  cancelTransfer: (id) => set((s) => ({ transfers: s.transfers.filter((t) => t.id !== id) })),

  sendFile: (name, size, to) =>
    set((s) => ({
      transfers: [
        { id: `t${Date.now()}`, name, size, from: s.me, to, bytesDone: 0, speed: 85_000_000, state: "active", resumedFrom: 0, updated: Date.now() },
        ...s.transfers,
      ],
    })),

  captureWorkspace: (name) =>
    set((s) => ({
      workspaces: [
        {
          id: `w${Date.now()}`,
          name,
          device: s.me,
          ts: Date.now(),
          tabs: [{ title: "Tether", url: "localhost:1420" }],
          files: ["app/src/data/store.ts"],
          terminals: ["~/Tether"],
        },
        ...s.workspaces,
      ],
    })),

  renameDevice: (id, name) => set((s) => ({ devices: s.devices.map((d) => (d.id === id ? { ...d, name } : d)) })),
  revokeDevice: (id) => set((s) => ({ devices: s.devices.filter((d) => d.id !== id) })),
}));

/**
 * Stand-in for the daemon: peers copy things, transfers progress and latency
 * wobbles. Remove this when the Tauri client is wired up.
 */
const NOMINAL_SPEED = 100_000_000;
const baseLatency = new Map<string, number>();

function wobbleLatency(d: Device): number {
  const base = baseLatency.get(d.id) ?? d.latencyMs ?? 1;
  baseLatency.set(d.id, base);
  return Math.max(0.8, +(base * (0.85 + Math.random() * 0.3)).toFixed(1));
}

export function startMockDaemon(): () => void {
  let copyIndex = 0;

  const copies = setInterval(() => {
    const c = mock.remoteCopies[copyIndex++ % mock.remoteCopies.length];
    const s = useTether.getState();
    const dev = s.devices.find((d) => d.id === c.device);
    if (!dev || dev.connection === "offline") return;
    s.addClip(c.device, c.text);
    s.toast(`Copied on ${dev.name}`);
  }, 14_000);

  let ticks = 0;
  const tick = setInterval(() => {
    // Latency is re-sampled every 2.5 s; transfers advance every 500 ms.
    const sampleLatency = ticks++ % 5 === 0;
    useTether.setState((s) => ({
      transfers: s.transfers.map((t) => {
        if (t.state !== "active") return t;
        // Pull toward a nominal LAN speed so the wobble never drifts away.
        const speed = Math.round(t.speed * 0.8 + NOMINAL_SPEED * (0.85 + Math.random() * 0.3) * 0.2);
        const bytesDone = Math.min(t.size, t.bytesDone + speed / 2);
        return bytesDone >= t.size ? { ...t, bytesDone, speed: 0, state: "done", updated: Date.now() } : { ...t, bytesDone, speed };
      }),
      devices: !sampleLatency ? s.devices : s.devices.map((d) =>
        d.connection === "offline" || d.isMe || d.latencyMs === null
          ? d
          : { ...d, latencyMs: wobbleLatency(d), lastSeen: Date.now() },
      ),
    }));
  }, 500);

  return () => {
    clearInterval(copies);
    clearInterval(tick);
  };
}
