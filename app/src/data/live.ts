// Inside the desktop app: fill the store from tetherd and keep it current.
// The daemon pushes events (new copies, device changes, pause); round trips
// are polled every few seconds while the window is visible.

import { os } from "@/lib/platform";
import { daemon, errorText, type DaemonEvent } from "./daemon";
import { applyItemChange, clipOf, devicesOf, heldBecause, itemOf, mergeClips, nearbyOf, shortId, withArticle } from "./map";
import { useTether, type Backend } from "./store";

/** Items fetched per history page (the daemon's maximum). */
const PAGE = 100;
/** How often round trips are refreshed while the window is visible. */
const RTT_POLL_MS = 3000;

const set = useTether.setState;
const get = useTether.getState;

function nameOf(id: string): string {
  return get().devices.find((d) => d.id === id)?.name ?? shortId(id);
}

async function refreshStatus() {
  const s = await daemon.status();
  set({ me: s.device, paused: s.paused, port: s.port });
}

async function refreshDevices() {
  const list = await daemon.devices();
  set((s) => ({ devices: devicesOf(list, os, s.devices), nearby: nearbyOf(list) }));
}

/** Reload the newest page. Older pages the user opened are dropped; they can load them again. */
async function refreshHistory() {
  const page = await daemon.history(null, PAGE);
  set({ clips: mergeClips([], page.items.map((i) => clipOf(i))), historyNext: page.next });
}

async function refreshAll() {
  try {
    // Status first: it tells us which device is "me".
    await refreshStatus();
    await Promise.all([refreshDevices(), refreshHistory()]);
    set({ link: { state: "connected" } });
  } catch (e) {
    set({ link: { state: "unreachable", reason: errorText(e) } });
  }
}

function onEvent(e: DaemonEvent) {
  const s = get();
  switch (e.type) {
    case "clipboard": {
      const clip = clipOf(e.item, true);
      set({ clips: mergeClips(s.clips, [clip]) });
      if (clip.device === s.me) break;
      if (clip.held) {
        s.toast(`Held a copy from ${nameOf(clip.device)}`, `It is in your history but not on your clipboard, because ${heldBecause(clip.held)}.`);
      } else if (!s.paused) {
        s.toast(`Copied on ${nameOf(clip.device)}`);
      }
      break;
    }
    case "historyChanged":
      void refreshHistory().catch(() => {});
      break;
    case "keptLocal":
      s.toast("Kept on this device", `That looked like ${withArticle(e.kind)}, so it was not synced.`);
      break;
    case "devicesChanged":
      void Promise.all([refreshDevices(), refreshStatus()]).catch(() => {});
      break;
    case "paused":
      set({ paused: e.paused });
      break;
    case "itemChanged":
      set({ clips: applyItemChange(s.clips, e) });
      break;
    case "lagged":
      void refreshAll();
      break;
  }
}

const liveBackend: Backend = {
  live: true,
  async revoke(id) {
    await daemon.revoke(id);
    await refreshDevices();
  },
  async setPaused(paused) {
    set({ paused: await daemon.setPaused(paused) });
  },
  pair: () => daemon.pair(),
  join: (code, addr) => daemon.join(code, addr),
  async loadOlder() {
    const before = get().historyNext;
    if (!before) return;
    const page = await daemon.history(before, PAGE);
    set((s) => ({ clips: mergeClips(s.clips, page.items.map((i) => clipOf(i))), historyNext: page.next }));
  },
  openLink(url) {
    daemon.openLink(url).catch((e) => get().toast("Could not open the link", errorText(e)));
  },
  async setAutostart(enabled) {
    set({ autostart: await daemon.setAutostart(enabled) });
  },
  // The daemon answers with an itemChanged event, which updates the list.
  async setPinned(clipId, pinned) {
    const item = itemOf(clipId);
    if (!item) throw new Error("That item is not in history");
    await daemon.setPinned(item.author, item.seq, pinned);
  },
  async deleteItem(clipId) {
    const item = itemOf(clipId);
    if (!item) throw new Error("That item is not in history");
    await daemon.deleteItem(item.author, item.seq);
  },
};

/** Switch the store from preview data to the daemon and follow it. */
export async function connectDaemon(): Promise<void> {
  set({
    backend: liveBackend,
    link: { state: "connecting" },
    me: "",
    devices: [],
    nearby: [],
    clips: [],
    historyNext: null,
    transfers: [],
    workspaces: [],
    ops: [],
  });
  // An app setting, not the daemon's: readable even before the daemon answers.
  daemon.autostart().then(
    (autostart) => set({ autostart }),
    () => set({ autostart: null }),
  );
  await daemon.onEvent(onEvent);
  await daemon.onLink((link) => {
    if (link.state === "connected") void refreshAll();
    else set({ link });
  });
  // The first `status` call also starts the daemon if none is running.
  await refreshAll();

  setInterval(() => {
    if (document.visibilityState === "visible" && get().link.state === "connected") void refreshDevices().catch(() => {});
  }, RTT_POLL_MS);
}
