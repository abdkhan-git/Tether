// The note editor: a plain textarea driven by a NoteSession, with list
// continuation, undo that only undoes what was typed here, and an optional
// layer that tints who wrote what. Shared by the Notes page and the floating
// scratchpad.

import { forwardRef, useEffect, useImperativeHandle, useMemo, useReducer, useRef, useState, type KeyboardEvent } from "react";
import { listen } from "@/data/notes";
import { movePosition, type Delta } from "@/lib/notes/delta";
import { NoteSession, visible, type Span, type Wire } from "@/lib/notes/session";
import { isMod } from "@/lib/platform";

/** One NoteSession for one note, following the daemon's events for it. */
export function useNoteSession(note: string, wire: Wire, me: string, area: React.RefObject<HTMLTextAreaElement | null>) {
  const [, redraw] = useReducer((n: number) => n + 1, 0);
  const [session, setSession] = useState<NoteSession | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  useEffect(() => {
    const s: NoteSession = new NoteSession(me, wire, {
      onText: (moved: Delta) => {
        const ta = area.current;
        if (ta) {
          const [start, end] = [ta.selectionStart, ta.selectionEnd];
          ta.value = visible(s.text);
          if (document.activeElement === ta) ta.setSelectionRange(movePosition(moved, start), movePosition(moved, end));
          grow(ta);
        }
        redraw();
      },
      onState: redraw,
      onNotice: setNotice,
    });
    const off = listen(note, { patch: (p) => s.patch(p), noop: (e, n) => s.noop(e, n), reopen: () => void s.reopen() });
    setSession(s);
    setNotice(null);
    if (area.current) area.current.value = "";
    void s.start().then(() => {
      const ta = area.current;
      if (ta && ta.value !== visible(s.text)) {
        ta.value = visible(s.text);
        grow(ta);
      }
      redraw();
    });
    // Send what is still unsent if the window goes away.
    const leave = () => s.flush();
    window.addEventListener("pagehide", leave);
    return () => {
      window.removeEventListener("pagehide", leave);
      off();
      s.close();
    };
  }, [note, wire, me, area]);
  return { session, notice, clearNotice: () => setNotice(null) };
}

/** Fit a textarea's height to its text, so the page scrolls instead. */
export function grow(ta: HTMLTextAreaElement, fill = false) {
  if (fill) return;
  ta.style.height = "auto";
  ta.style.height = `${ta.scrollHeight}px`;
}

const LIST = /^(\s*)([-*+]|(\d{1,9})([.)]))(\s+)(\[[ xX]\]\s+)?/;

export interface EditorHandle {
  focus(): void;
}

interface Props {
  session: NoteSession | null;
  /** Device id to tint index, when showing who wrote what. */
  tints?: Map<string, number> | null;
  placeholder?: string;
  /** Fill the parent (the scratchpad) instead of growing with the text. */
  fill?: boolean;
  className?: string;
  label: string;
}

/** The textarea, wired to a session. */
export const NoteTextarea = forwardRef<HTMLTextAreaElement, Props>(function NoteTextarea({ session, tints, placeholder, fill, className, label }, ref) {
  const area = useRef<HTMLTextAreaElement>(null);
  useImperativeHandle(ref, () => area.current!, []);
  const composing = useRef(false);

  useEffect(() => {
    if (area.current) grow(area.current, fill);
  });

  const ta = () => area.current!;

  /** Replace part of the text from here, putting the caret after what was inserted. */
  const replace = (at: number, remove: number, insert: string, select?: [number, number]) => {
    if (!session) return;
    session.replace(at, remove, insert);
    const el = ta();
    const [a, b] = select ?? [at + insert.length, at + insert.length];
    el.setSelectionRange(a, b);
  };

  const afterUndo = (d: Delta | null) => {
    if (!d || !session) return;
    const el = ta();
    const caret = movePosition(d, el.selectionStart, true);
    el.value = visible(session.text);
    el.setSelectionRange(caret, caret);
    grow(el, fill);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (!session || composing.current) return;
    const el = ta();
    const { selectionStart: start, selectionEnd: end, value } = el;
    const key = e.key.toLowerCase();
    if (isMod(e) && key === "z" && !e.shiftKey) {
      e.preventDefault();
      afterUndo(session.undo());
    } else if (isMod(e) && (key === "y" || (key === "z" && e.shiftKey))) {
      e.preventDefault();
      afterUndo(session.redo());
    } else if (isMod(e) && (key === "b" || key === "i")) {
      e.preventDefault();
      const mark = key === "b" ? "**" : "*";
      const inner = value.slice(start, end);
      replace(start, end - start, `${mark}${inner}${mark}`, [start + mark.length, end + mark.length]);
    } else if (isMod(e) && e.key === "Enter") {
      // Tick or untick the checklist item the caret is on.
      e.preventDefault();
      const lineStart = value.lastIndexOf("\n", start - 1) + 1;
      const m = LIST.exec(value.slice(lineStart));
      if (m?.[6]) {
        const box = lineStart + m[1].length + m[2].length + m[5].length;
        replace(box, 3, m[6].startsWith("[ ]") ? "[x]" : "[ ]", [start, end]);
      }
    } else if (e.key === "Tab") {
      e.preventDefault();
      if (e.shiftKey) {
        const lineStart = value.lastIndexOf("\n", start - 1) + 1;
        const spaces = /^ {1,2}/.exec(value.slice(lineStart))?.[0].length ?? 0;
        if (spaces) replace(lineStart, spaces, "", [Math.max(lineStart, start - spaces), Math.max(lineStart, end - spaces)]);
      } else replace(start, end - start, "  ");
    } else if (e.key === "Enter" && !e.shiftKey && start === end) {
      const lineStart = value.lastIndexOf("\n", start - 1) + 1;
      const m = LIST.exec(value.slice(lineStart, start));
      if (!m) return;
      e.preventDefault();
      if (start - lineStart === m[0].length) {
        replace(lineStart, start - lineStart, ""); // an empty item ends the list
      } else {
        const marker = m[3] ? `${Number(m[3]) + 1}${m[4]}` : m[2];
        replace(start, 0, `\n${m[1]}${marker}${m[5]}${m[6] ? "[ ] " : ""}`);
      }
    }
  };

  const layer = useMemo(() => {
    if (!tints || !session) return null;
    return spansOf(visible(session.text), session.authors, tints);
    // The session object is the same between renders; its text is what changes.
  }, [tints, session, session?.text]); // eslint-disable-line react-hooks/exhaustive-deps

  const readOnly = !session || session.status === "opening" || session.status === "failed";
  return (
    <div className={`note-stack ${fill ? "h-full" : ""} ${className ?? ""}`}>
      {layer && (
        <div className="note-layer" aria-hidden>
          {layer}
          {"\n"}
        </div>
      )}
      <textarea
        ref={area}
        aria-label={label}
        placeholder={placeholder}
        readOnly={readOnly}
        spellCheck
        className={fill ? "h-full" : undefined}
        onInput={(e) => {
          if (!composing.current && session) {
            session.typed(e.currentTarget.value, e.currentTarget.selectionEnd);
            grow(e.currentTarget, fill);
          }
        }}
        onBeforeInput={(e) => {
          const type = (e.nativeEvent as InputEvent).inputType;
          if (session && (type === "historyUndo" || type === "historyRedo")) {
            e.preventDefault();
            afterUndo(type === "historyUndo" ? session.undo() : session.redo());
          }
        }}
        onCompositionStart={() => {
          composing.current = true;
          session?.composing(true);
        }}
        onCompositionEnd={(e) => {
          composing.current = false;
          session?.typed(e.currentTarget.value, e.currentTarget.selectionEnd);
          session?.composing(false);
        }}
        onKeyDown={onKeyDown}
        onBlur={() => session?.flush()}
      />
    </div>
  );
});

/** The text cut into tinted runs by author. */
function spansOf(text: string, authors: Span[], tints: Map<string, number>) {
  const out: React.ReactNode[] = [];
  let at = 0;
  authors.forEach((s, i) => {
    out.push(
      <span key={i} className={`author-${tints.get(s.device) ?? 5}`}>
        {text.slice(at, at + s.chars)}
      </span>,
    );
    at += s.chars;
  });
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/** Tint indexes for the devices that wrote a note: this one first, then by how much each wrote. */
export function tintsFor(authors: Span[], me: string): Map<string, number> {
  const amount = new Map<string, number>();
  for (const s of authors) amount.set(s.device, (amount.get(s.device) ?? 0) + s.chars);
  const others = [...amount.keys()].filter((d) => d !== me).sort((a, b) => amount.get(b)! - amount.get(a)!);
  return new Map([[me, 0], ...others.map((d, i): [string, number] => [d, Math.min(5, i + 1)])]);
}
