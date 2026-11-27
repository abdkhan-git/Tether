import { useEffect, useMemo, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { daemon, errorText, inDesktopApp } from "@/data/daemon";
import { plainPreview, SCRATCHPAD, titleOf, useNotes, type NoteSummary, type Revision } from "@/data/notes";
import { useTether } from "@/data/store";
import { counts } from "@/lib/notes/markdown";
import type { OpenedNote, Span } from "@/lib/notes/session";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/cn";
import { Icon, icons } from "@/components/Icon";
import { Button, Card, Chip, EmptyState, IconButton, Kbd, PageHeader, Panel, PrimaryAction, Segmented } from "@/components/ui";
import type { TetherModule } from "../types";
import { NoteTextarea, tintsFor, useNoteSession } from "./editor";
import { MarkdownView } from "./markdown-view";
import * as mock from "@/data/mock";

const spring = { type: "spring", duration: 0.45, bounce: 0.12 } as const;

type View = "write" | "read" | "history";

function useMe(): string {
  const me = useTether((s) => s.me);
  return inDesktopApp ? me : mock.ME;
}

function useNames(): (id: string) => string {
  const devices = useTether((s) => s.devices);
  const me = useMe();
  return useMemo(() => {
    const names = new Map(devices.map((d) => [d.id, d.name]));
    return (id: string) => (id === me ? "This device" : (names.get(id) ?? "A removed device"));
  }, [devices, me]);
}

function Status({ status }: { status: string }) {
  const label: Record<string, string> = { opening: "Opening…", saved: "Saved on this device", saving: "Saving…", offline: "Can't reach Tether; still typing here", secret: "Waiting for you", failed: "Can't open" };
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
      <span className={cn("size-1.5 rounded-full", status === "saved" ? "bg-positive" : "bg-subtle")} />
      {label[status] ?? status}
    </span>
  );
}

function Legend({ authors, tints, name }: { authors: Span[]; tints: Map<string, number>; name: (id: string) => string }) {
  const devices = [...new Set(authors.map((a) => a.device))];
  return (
    <span className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
      {devices.map((d) => (
        <span key={d} className="inline-flex items-center gap-1.5">
          <span className={`author-dot author-${tints.get(d) ?? 5}`} />
          {name(d)}
        </span>
      ))}
    </span>
  );
}

function History({ note, onRestored }: { note: string; onRestored: () => void }) {
  const backend = useNotes((s) => s.backend);
  const toast = useTether((s) => s.toast);
  const name = useNames();
  const me = useMe();
  const [revisions, setRevisions] = useState<Revision[] | null>(null);
  const [picked, setPicked] = useState<Revision | null>(null);
  const [text, setText] = useState<OpenedNote | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    backend.history(note).then(
      (list) => {
        setRevisions(list);
        setPicked(list[0] ?? null);
      },
      (e) => toast("Could not load the history", errorText(e)),
    );
  }, [backend, note, toast]);

  useEffect(() => {
    setConfirming(false);
    if (!picked) return setText(null);
    backend.at(note, picked).then(setText, (e) => toast("Could not load that version", errorText(e)));
  }, [backend, note, picked, toast]);

  const restore = () => {
    if (!picked) return;
    backend.restore(note, picked).then(
      () => {
        toast("Restored", "Saved as a new change, so this can be undone from the history too.");
        onRestored();
      },
      (e) => toast("Could not restore", errorText(e)),
    );
  };

  if (!revisions) return <p className="py-10 text-center text-[13px] text-subtle">Loading the history…</p>;
  if (!revisions.length) return <EmptyState title="No history yet" hint="Every change is kept, on every device. Start writing and it shows up here." />;
  const tints = text ? tintsFor(text.authors, me) : null;
  return (
    <div className="grid gap-4 md:grid-cols-[230px_minmax(0,1fr)]">
      <ul className="flex max-h-[56vh] flex-col gap-0.5 overflow-y-auto pr-1" aria-label="Versions">
        {revisions.map((r, i) => (
          <li key={`${r.at}:${r.counter}:${r.author}`}>
            <button onClick={() => setPicked(r)} className={cn("w-full rounded-xl px-3 py-2 text-left", picked === r ? "bg-muted" : "hover:bg-muted/60")}>
              <span className="block text-[13px] text-foreground">{i === 0 ? "Latest" : timeAgo(r.at)}</span>
              <span className="block truncate text-xs text-subtle">
                {name(r.author)} · +{r.inserted} −{r.deleted}
              </span>
            </button>
          </li>
        ))}
      </ul>
      <div className="min-w-0">
        {text && tints && (
          <>
            <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
              <Legend authors={text.authors} tints={tints} name={name} />
              {picked !== revisions[0] &&
                (confirming ? (
                  <span className="flex items-center gap-1.5">
                    <Button size="sm" variant="ghost" onClick={() => setConfirming(false)}>
                      Cancel
                    </Button>
                    <Button size="sm" variant="solid" icon={icons.restore} onClick={restore}>
                      Restore on every device
                    </Button>
                  </span>
                ) : (
                  <Button size="sm" variant="outline" icon={icons.restore} onClick={() => setConfirming(true)}>
                    Restore this version
                  </Button>
                ))}
            </div>
            <div className="max-h-[52vh] overflow-y-auto rounded-2xl border border-border bg-surface-2 p-4">
              <div className="note-stack">
                <div className="note-layer selectable !text-foreground">
                  {spans(text.text, text.authors, tints)}
                </div>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function spans(text: string, authors: Span[], tints: Map<string, number>) {
  // Authors count characters as the daemon does; slice by characters.
  const chars = [...text];
  let at = 0;
  return authors.map((s, i) => {
    const part = chars.slice(at, at + s.chars).join("");
    at += s.chars;
    return (
      <span key={i} className={`author-${tints.get(s.device) ?? 5}`}>
        {part}
      </span>
    );
  });
}

function NotePane({ note, notebooks }: { note: NoteSummary; notebooks: string[] }) {
  const backend = useNotes((s) => s.backend);
  const toast = useTether((s) => s.toast);
  const me = useMe();
  const name = useNames();
  const area = useRef<HTMLTextAreaElement>(null);
  const wire = useMemo(() => backend.wire(note.id), [backend, note.id]);
  const { session, notice, clearNotice } = useNoteSession(note.id, wire, me, area);
  const [view, setView] = useState<View>("write");
  const [showAuthors, setShowAuthors] = useState(false);
  const [title, setTitle] = useState(note.title);
  const [notebook, setNotebook] = useState(note.notebook);
  const scratch = note.id === SCRATCHPAD;

  useEffect(() => setTitle(note.title), [note.title]);
  useEffect(() => setNotebook(note.notebook), [note.notebook]);

  const meta = (change: Parameters<typeof backend.meta>[1], done?: string) =>
    backend.meta(note.id, change).then(
      () => done && toast(done),
      (e) => toast("Could not change the note", errorText(e)),
    );
  const commitTitle = () => title.trim() !== note.title && void meta({ title: title.trim() });
  const commitNotebook = () => notebook.trim() !== note.notebook && void meta({ notebook: notebook.trim() });

  const text = session?.text ?? "";
  const tints = useMemo(() => (showAuthors && session ? tintsFor(session.authors, me) : null), [showAuthors, session, session?.authors, me]); // eslint-disable-line react-hooks/exhaustive-deps
  const { words, chars } = counts(text);

  return (
    <Card pad={28} className="flex min-h-[520px] flex-col gap-5">
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          {scratch ? (
            <h2 className="text-[22px] font-semibold tracking-tight">Scratchpad</h2>
          ) : (
            <input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              onBlur={commitTitle}
              onKeyDown={(e) => e.key === "Enter" && (e.currentTarget.blur(), area.current?.focus())}
              placeholder={titleOf({ ...note, title: "" })}
              maxLength={200}
              aria-label="Title"
              className="selectable w-full bg-transparent text-[22px] font-semibold tracking-tight outline-none placeholder:text-subtle"
            />
          )}
          <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1.5">
            {session && <Status status={session.status} />}
            {!scratch && (
              <label className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                <Icon icon={icons.notebook} size={13} />
                <input
                  value={notebook}
                  onChange={(e) => setNotebook(e.target.value)}
                  onBlur={commitNotebook}
                  onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  list="tether-notebooks"
                  placeholder="No notebook"
                  maxLength={64}
                  aria-label="Notebook"
                  className="selectable w-32 bg-transparent outline-none placeholder:text-subtle"
                />
                <datalist id="tether-notebooks">
                  {notebooks.map((n) => (
                    <option key={n} value={n} />
                  ))}
                </datalist>
              </label>
            )}
            <span className="text-xs text-subtle">
              Changed {timeAgo(note.updated)} {note.updatedBy === me ? "here" : `on ${name(note.updatedBy)}`}
            </span>
          </div>
        </div>
        <div className="flex items-center gap-1">
          <Segmented
            label="View"
            value={view}
            onChange={setView}
            options={[
              { value: "write", label: "Write" },
              { value: "read", label: "Read" },
              { value: "history", label: "History" },
            ]}
          />
          <IconButton icon={icons.people} label={showAuthors ? "Hide who wrote what" : "Show who wrote what"} onClick={() => setShowAuthors(!showAuthors)} className={showAuthors ? "bg-muted text-foreground" : undefined} />
          {!scratch && <IconButton icon={note.pinned ? icons.pinOff : icons.pin} label={note.pinned ? "Unpin" : "Pin"} onClick={() => void meta({ pinned: !note.pinned })} />}
          {!scratch && !note.trashed && <IconButton icon={icons.trash} label="Move to trash" tone="danger" onClick={() => void meta({ trashed: true }, "Moved to the trash on every device")} />}
        </div>
      </div>

      {note.trashed && (
        <div className="-mt-1 flex items-center justify-between gap-3 rounded-2xl border border-border bg-surface-2 px-4 py-3 text-[13px] text-muted-foreground">
          <span>In the trash on every device. Its history is kept.</span>
          <Button size="sm" variant="outline" icon={icons.restore} onClick={() => void meta({ trashed: false }, "Taken out of the trash")}>
            Take out of the trash
          </Button>
        </div>
      )}
      {session?.status === "secret" && (
        <div role="alert" className="-mt-1 rounded-2xl border border-border bg-surface-2 px-4 py-3 text-[13px] text-muted-foreground">
          <p>
            <span className="font-medium text-foreground">That looks like {/^[aeiou]/i.test(session.secret ?? "") ? "an" : "a"} {session.secret}.</span> Notes go to every device in your group and keep their history, so it would stay
            there even after you delete it. Nothing has been saved yet.
          </p>
          <div className="mt-2.5 flex gap-1.5">
            <Button size="sm" variant="solid" onClick={() => session.answerSecret(false)}>
              Take it out
            </Button>
            <Button size="sm" variant="ghost" onClick={() => session.answerSecret(true)}>
              Keep it in the note
            </Button>
          </div>
        </div>
      )}
      {notice && (
        <div role="alert" className="-mt-1 flex items-center justify-between gap-3 rounded-2xl border border-border bg-surface-2 px-4 py-3 text-[13px] text-muted-foreground">
          <span>{notice}. That change was taken back.</span>
          <IconButton icon={icons.close} label="Dismiss" onClick={clearNotice} />
        </div>
      )}
      {session?.status === "failed" && <p className="text-[13px] text-muted-foreground">Can't open this note: {session.error}</p>}

      <div className="min-h-0 flex-1">
        <div className={view === "write" ? "" : "hidden"}>
          <NoteTextarea ref={area} session={session} tints={tints} label="Note" placeholder={scratch ? "Jot something down. It is on every device as you type." : "Write in Markdown: # headings, - lists, - [ ] checklists, `code`."} />
        </div>
        {view === "read" && <MarkdownView text={text} onToggle={(box, replacement) => session?.replace(box, 3, replacement)} />}
        {view === "history" && <History note={note.id} onRestored={() => setView("write")} />}
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-border pt-4 text-xs text-subtle">
        {tints && session ? <Legend authors={session.authors} tints={tints} name={name} /> : <span>{view === "write" ? "Ctrl+Enter ticks a checklist item · Ctrl+Z undoes only what you typed here" : ""}</span>}
        <span className="tabular-nums">
          {words} words · {chars} characters
        </span>
      </div>
    </Card>
  );
}

type Filter = "all" | "pinned" | "trash" | `book:${string}`;

function NotesView() {
  const { list, loaded, refresh, backend, error } = useNotes();
  const toast = useTether((s) => s.toast);
  const scratchpadShortcut = useTether((s) => s.scratchpadShortcut);
  const name = useNames();
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => void refresh(), [refresh]);

  const notebooks = useMemo(() => [...new Set(list.map((n) => n.notebook).filter(Boolean))].sort(), [list]);
  const scratch = list.find((n) => n.id === SCRATCHPAD);
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return list
      .filter((n) => n.id !== SCRATCHPAD)
      .filter((n) => (filter === "trash" ? n.trashed : !n.trashed))
      .filter((n) => (filter === "pinned" ? n.pinned : filter.startsWith("book:") ? n.notebook === filter.slice(5) : true))
      .filter((n) => !q || `${n.title} ${n.preview} ${n.notebook}`.toLowerCase().includes(q))
      .sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updated - a.updated);
  }, [list, filter, query]);
  const selected = list.find((n) => n.id === selectedId) ?? visible[0] ?? scratch ?? null;

  const newNote = () =>
    backend.create("", filter.startsWith("book:") ? filter.slice(5) : "").then(
      (id) => {
        setSelectedId(id);
        if (filter === "trash") setFilter("all");
        void refresh();
      },
      (e) => toast("Could not create a note", errorText(e)),
    );

  const filters: { value: Filter; label: string }[] = [
    { value: "all", label: "All" },
    { value: "pinned", label: "Pinned" },
    ...notebooks.map((b) => ({ value: `book:${b}` as Filter, label: b })),
    { value: "trash", label: "Trash" },
  ];

  return (
    <div>
      <PageHeader
        eyebrow="Notes"
        title="Write once, on every computer"
        subtitle="Notes merge as you type on several devices, offline included. Every change is kept, with who made it."
        actions={
          <PrimaryAction icon={icons.add} onClick={() => void newNote()}>
            New note
          </PrimaryAction>
        }
      />
      <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,320px)_minmax(0,1fr)]">
        <Panel className="flex max-h-[calc(100vh-260px)] min-h-[520px] flex-col p-2">
          <label className="mx-1 mt-1 mb-2 flex h-9 items-center gap-2 rounded-full bg-muted px-3.5 text-muted-foreground">
            <Icon icon={icons.search} size={15} />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search notes" aria-label="Search notes" className="selectable min-w-0 flex-1 bg-transparent text-[13px] text-foreground outline-none placeholder:text-subtle" />
          </label>
          <div className="mx-1 mb-2 flex flex-wrap gap-1" role="tablist" aria-label="Show">
            {filters.map((f) => (
              <button key={f.value} role="tab" aria-selected={filter === f.value} onClick={() => setFilter(f.value)} className={cn("rounded-full px-2.5 py-1 text-xs transition-colors", filter === f.value ? "bg-foreground text-background" : "text-muted-foreground hover:bg-muted")}>
                {f.label}
              </button>
            ))}
          </div>
          {scratch && filter !== "trash" && (
            <div className={cn("mx-0.5 mb-1 flex items-center gap-2 rounded-2xl px-3 py-2.5", selected?.id === SCRATCHPAD ? "bg-muted" : "hover:bg-muted/60")}>
              <button onClick={() => setSelectedId(SCRATCHPAD)} className="flex min-w-0 flex-1 items-center gap-3 text-left">
                <span className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-surface-2 text-muted-foreground">
                  <Icon icon={icons.sticky} size={15} />
                </span>
                <span className="min-w-0">
                  <span className="block text-[13.5px] font-medium">Scratchpad</span>
                  <span className="block truncate text-xs text-subtle">{plainPreview(scratch.preview) || "Quick notes, floating over any app"}</span>
                </span>
              </button>
              {inDesktopApp && (
                <IconButton icon={icons.external} label="Float over other apps" shortcut={scratchpadShortcut || undefined} onClick={() => void daemon.showScratchpad()} />
              )}
            </div>
          )}
          <div className="min-h-0 flex-1 overflow-y-auto">
            {!loaded ? (
              <p className="py-10 text-center text-[13px] text-subtle">Loading…</p>
            ) : error ? (
              <EmptyState title="Can't load notes" hint={error} />
            ) : visible.length === 0 ? (
              <EmptyState title={filter === "trash" ? "The trash is empty" : query ? "Nothing matches" : "No notes yet"} hint={filter === "trash" ? undefined : "Start one with New note. It appears on your other computers as you type."} />
            ) : (
              <ul className="flex flex-col gap-0.5" aria-label="Notes">
                <AnimatePresence initial={false}>
                  {visible.map((n) => (
                    <motion.li key={n.id} layout="position" initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, height: 0 }} transition={spring}>
                      <button onClick={() => setSelectedId(n.id)} className="relative w-full rounded-2xl px-3 py-2.5 text-left">
                        {selected?.id === n.id && <motion.span layoutId="note-selected" className="absolute inset-0 rounded-2xl bg-muted" transition={spring} />}
                        <span className="relative block">
                          <span className="flex items-center gap-1.5">
                            <span className="truncate text-[13.5px] font-medium">{titleOf(n)}</span>
                            {n.pinned && <Icon icon={icons.pin} size={12} className="shrink-0 text-subtle" />}
                          </span>
                          <span className="mt-0.5 line-clamp-2 text-xs text-muted-foreground [unicode-bidi:isolate]">{plainPreview(n.preview) || "Empty"}</span>
                          <span className="mt-1 block truncate text-[11px] text-subtle">
                            {timeAgo(n.updated)} · {name(n.updatedBy)}
                            {n.notebook ? ` · ${n.notebook}` : ""}
                          </span>
                        </span>
                      </button>
                    </motion.li>
                  ))}
                </AnimatePresence>
              </ul>
            )}
          </div>
        </Panel>

        {selected ? (
          <NotePane key={selected.id} note={selected} notebooks={notebooks} />
        ) : (
          <Card className="min-h-[520px]">
            <EmptyState title="No note open" hint="Pick a note, or start one." />
          </Card>
        )}
      </div>
      {scratchpadShortcut && (
        <p className="mt-4 flex items-center gap-2 px-1 text-xs text-subtle">
          <Kbd>{scratchpadShortcut}</Kbd> opens the scratchpad over any app. <Chip>Notes are signed and merged like everything else in Tether</Chip>
        </p>
      )}
    </div>
  );
}

export const notesModule: TetherModule = {
  id: "notes",
  label: "Notes",
  icon: icons.note,
  group: "Workspace",
  summary: "Notes that merge live across your computers, offline included, with history and who wrote what.",
  status: "preview",
  note: "F11",
  daemon: true,
  view: NotesView,
  commands: () => [
    {
      id: "notes-new",
      label: "New note",
      keywords: "note write create markdown",
      run: () => {
        const s = useNotes.getState();
        void s.backend.create("", "").then(() => void s.refresh());
        useTether.getState().setActiveModule("notes");
      },
    },
    ...(inDesktopApp ? [{ id: "notes-scratchpad", label: "Open the scratchpad", keywords: "scratch pad note float quick", run: () => void daemon.showScratchpad() }] : []),
  ],
};
