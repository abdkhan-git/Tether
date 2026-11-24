// The quick paste palette: a small window that opens over any app with
// Alt+Shift+V. Type to search history from every device, press Enter to put
// the item on this device's clipboard, Esc to close.
//
// It runs in its own window with its own, narrower capability
// (src-tauri/capabilities/palette.json): read history and device names, copy
// an item here, hide itself. In a plain browser (`?palette`) it shows the
// preview data, so it can be designed without the app.

import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { daemon, errorText, inDesktopApp } from "@/data/daemon";
import { clipOf, devicesOf, shortId } from "@/data/map";
import * as mock from "@/data/mock";
import { newestFirst, type Clip } from "@/data/types";
import { matches } from "./search";
import { contentOf } from "@/lib/content/detect";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { os } from "@/lib/platform";
import { Icon, icons } from "@/components/Icon";
import { kindIcon } from "@/modules/clipboard";

/** Most results shown at once. */
const SHOWN = 50;

interface Loaded {
  clips: Clip[];
  names: Map<string, string>;
  me: string;
}

async function load(): Promise<Loaded> {
  if (!inDesktopApp) {
    return { clips: [...mock.clips].sort(newestFirst), names: new Map(mock.devices.map((d) => [d.id, d.name])), me: mock.ME };
  }
  const [status, list, page] = await Promise.all([daemon.status(), daemon.devices(), daemon.history(null, 100)]);
  const devices = devicesOf(list, os, []);
  return { clips: page.items.map((i) => clipOf(i)).sort(newestFirst), names: new Map(devices.map((d) => [d.id, d.name])), me: status.device };
}

async function hide() {
  if (!inDesktopApp) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().hide();
}

function Palette() {
  const [data, setData] = useState<Loaded>({ clips: [], names: new Map(), me: "" });
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const [note, setNote] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);

  const refresh = useCallback(() => {
    setQuery("");
    setIndex(0);
    setNote(null);
    input.current?.focus();
    load().then(setData, (e) => setNote(`Can't reach the Tether service: ${errorText(e)}`));
  }, []);

  useEffect(() => {
    refresh();
    if (!inDesktopApp) return;
    const off = daemon.onPaletteOpen(refresh);
    return () => void off.then((f) => f());
  }, [refresh]);

  const from = useCallback((c: Clip) => (c.device === data.me ? "This device" : (data.names.get(c.device) ?? shortId(c.device))), [data]);
  const results = useMemo(() => data.clips.filter((c) => matches(c, from(c), query)).slice(0, SHOWN), [data, query, from]);
  const selected = results[Math.min(index, results.length - 1)];

  const choose = async (clip: Clip | undefined) => {
    if (!clip) return;
    try {
      if (inDesktopApp) {
        const [author, seq] = clip.id.split(":");
        await daemon.copyHere(author, Number(seq));
      } else {
        await navigator.clipboard.writeText(clip.text);
      }
      await hide();
    } catch (e) {
      setNote(`Could not copy: ${errorText(e)}`);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.max(0, Math.min(results.length - 1, index + (e.key === "ArrowDown" ? 1 : -1)));
      setIndex(next);
      list.current?.querySelector(`[data-i="${next}"]`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter") {
      e.preventDefault();
      void choose(selected);
    } else if (e.key === "Escape") {
      e.preventDefault();
      void hide();
    }
  };

  return (
    <div className="flex h-screen flex-col overflow-hidden rounded-[18px] border border-border bg-surface text-foreground" onKeyDown={onKeyDown}>
      <div className="flex items-center gap-3 border-b border-border px-5 py-4">
        <Icon icon={icons.search} size={18} className="text-muted-foreground" />
        <input
          ref={input}
          autoFocus
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setIndex(0);
          }}
          placeholder="Search everything you copied, on every device"
          aria-label="Search clipboard history"
          spellCheck={false}
          className="min-w-0 flex-1 bg-transparent text-[17px] outline-none placeholder:text-subtle"
        />
      </div>

      <ul ref={list} className="min-h-0 flex-1 overflow-y-auto p-2" aria-label="Results">
        {results.map((c, i) => {
          const content = contentOf(c.text);
          const mono = !c.sensitive && (content.kind === "code" || content.kind === "json");
          return (
            <li key={c.id} data-i={i}>
              <button
                onMouseMove={() => setIndex(i)}
                onClick={() => void choose(c)}
                className={cn("flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left", i === index && "bg-muted")}
              >
                <span className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-2 text-muted-foreground">
                  <Icon icon={kindIcon[content.kind]} size={15} />
                </span>
                <span className="min-w-0 flex-1">
                  <span className={cn("block truncate text-[14px]", mono && "font-mono text-[12.5px]")}>{c.sensitive ? "•••••••••••••" : c.text.trim().split("\n")[0]}</span>
                  <span className="mt-0.5 block truncate text-xs text-subtle">
                    {content.label} · {from(c)} · {timeAgo(c.ts)}
                    {c.held ? " · held" : ""}
                    {c.pinned ? " · pinned" : ""}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
        {results.length === 0 && <li className="px-3 py-10 text-center text-[13px] text-subtle">{note ?? (query ? "Nothing matches." : "Nothing copied yet.")}</li>}
      </ul>

      <div className="flex items-center justify-between border-t border-border px-5 py-2.5 text-[11.5px] text-subtle">
        <span>{note && results.length ? note : "↑↓ to choose · Enter copies it here · Esc closes"}</span>
        <span className="font-mono">Alt+Shift+V</span>
      </div>
    </div>
  );
}

/** Render the palette into #root (instead of the main app). */
export function mountPalette() {
  try {
    const theme = JSON.parse(localStorage.getItem("tether.theme") ?? '"dark"');
    document.documentElement.setAttribute("data-theme", theme === "light" ? "light" : "dark");
  } catch {
    document.documentElement.setAttribute("data-theme", "dark");
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <Palette />
    </StrictMode>,
  );
}
