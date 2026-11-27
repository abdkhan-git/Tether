// The floating scratchpad: a small window over any app (Alt+Shift+N), holding
// the one note every device shares from the start. It runs in its own window
// with its own capability (src-tauri/capabilities/scratchpad.json): it can
// open and type in the scratchpad note and nothing else, and hears only that
// note's events. In a plain browser (`?scratchpad`) it uses the preview note.

import { StrictMode, useEffect, useMemo, useRef } from "react";
import { createRoot } from "react-dom/client";
import { inDesktopApp, daemon, type DaemonEvent } from "@/data/daemon";
import { noteEvents, scratchpadWire, SCRATCHPAD } from "@/data/notes";
import { Icon, icons } from "@/components/Icon";
import { NoteTextarea, useNoteSession } from "@/modules/notes/editor";

async function hide() {
  if (!inDesktopApp) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().hide();
}

function Scratchpad() {
  const area = useRef<HTMLTextAreaElement>(null);
  const wire = useMemo(() => scratchpadWire(), []);
  // Who wrote what is not shown here, so this device's id is not needed.
  const { session, notice, clearNotice } = useNoteSession(SCRATCHPAD, wire, "", area);

  useEffect(() => {
    area.current?.focus();
    if (!inDesktopApp) return;
    const offs = [
      daemon.onScratchpadOpen(() => area.current?.focus()),
      daemon.onEvent((e: DaemonEvent) => {
        if (e.type === "notePatch" && e.note === SCRATCHPAD) noteEvents.patch(SCRATCHPAD, e);
        else if (e.type === "noteNoop" && e.note === SCRATCHPAD) noteEvents.noop(SCRATCHPAD, e.editor, e.n);
        else if (e.type === "lagged") noteEvents.reopenAll();
      }),
      daemon.onLink((link) => link.state === "connected" && noteEvents.reopenAll()),
    ];
    return () => offs.forEach((off) => void off.then((f) => f()));
  }, []);

  const status = session?.status ?? "opening";
  return (
    <div
      className="scratch flex h-screen flex-col overflow-hidden rounded-[16px] border border-border bg-surface text-foreground"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          session?.flush();
          void hide();
        }
      }}
    >
      <header className="flex shrink-0 cursor-default items-center gap-2 border-b border-border px-4 py-2.5" data-tauri-drag-region>
        <Icon icon={icons.sticky} size={15} className="pointer-events-none text-muted-foreground" />
        <span className="pointer-events-none text-[13px] font-medium" data-tauri-drag-region>
          Scratchpad
        </span>
        <span className={`pointer-events-none size-1.5 rounded-full ${status === "saved" ? "bg-positive" : "bg-subtle"}`} title={status} />
        <span className="flex-1" data-tauri-drag-region />
        <button onClick={() => (session?.flush(), void hide())} aria-label="Put away" className="rounded-full p-1 text-muted-foreground hover:bg-muted hover:text-foreground">
          <Icon icon={icons.close} size={15} />
        </button>
      </header>
      {session?.status === "secret" && (
        <div role="alert" className="border-b border-border px-4 py-2.5 text-[12.5px] text-muted-foreground">
          That looks like {/^[aeiou]/i.test(session.secret ?? "") ? "an" : "a"} {session.secret}. It would go to every device.{" "}
          <button className="font-medium text-foreground underline" onClick={() => session.answerSecret(false)}>
            Take it out
          </button>{" "}
          ·{" "}
          <button className="underline" onClick={() => session.answerSecret(true)}>
            Keep it
          </button>
        </div>
      )}
      {notice && (
        <button role="alert" onClick={clearNotice} className="border-b border-border px-4 py-2 text-left text-[12.5px] text-muted-foreground">
          {notice}. That change was taken back.
        </button>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">
        <NoteTextarea ref={area} session={session} fill label="Scratchpad" placeholder="Jot something down. It is on your other computers as you type." className="h-full" />
      </div>
      <footer className="flex shrink-0 justify-between border-t border-border px-4 py-1.5 text-[11px] text-subtle">
        <span>{status === "offline" ? "Can't reach Tether; still typing here" : status === "failed" ? `Can't open: ${session?.error ?? ""}` : "On every device as you type"}</span>
        <span className="font-mono">Esc puts it away</span>
      </footer>
    </div>
  );
}

/** Render the scratchpad into #root (instead of the main app). */
export function mountScratchpad() {
  try {
    const theme = JSON.parse(localStorage.getItem("tether.theme") ?? '"dark"');
    document.documentElement.setAttribute("data-theme", theme === "light" ? "light" : "dark");
  } catch {
    document.documentElement.setAttribute("data-theme", "dark");
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <Scratchpad />
    </StrictMode>,
  );
}
