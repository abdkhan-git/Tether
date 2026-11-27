// Notes for the page: the list, and the way an editor reaches its note.
// Inside the desktop app this is the daemon (live.ts routes its note events
// here); in a plain browser it is a stand-in with sample notes, so the page
// can be designed without the app.

import { create } from "zustand";
import { daemon, inDesktopApp, type NoteMetaChange, type NoteSummary, type Revision } from "./daemon";
import { splice, type Delta } from "@/lib/notes/delta";
import { FakeNote } from "@/lib/notes/fake-daemon";
import type { EditRequest, EditResult, OpenedNote, Patch, Wire } from "@/lib/notes/session";
import * as mock from "./mock";

export type { NoteSummary, Revision };

export const SCRATCHPAD = "scratchpad";

/** What an open editor hears about its note. */
export interface NoteListener {
  patch(p: Patch): void;
  noop(editor: string, n: number): void;
  /** Events may have been missed: fetch the text again. */
  reopen(): void;
}

/** Everything the Notes page asks of whoever holds the notes. */
export interface NotesBackend {
  list(): Promise<NoteSummary[]>;
  create(title: string, notebook: string): Promise<string>;
  wire(note: string): Wire;
  meta(note: string, change: NoteMetaChange): Promise<void>;
  history(note: string): Promise<Revision[]>;
  at(note: string, revision: Revision): Promise<OpenedNote>;
  restore(note: string, revision: Revision): Promise<void>;
}

// --- Routing events to open editors ---

const listeners = new Map<string, Set<NoteListener>>();

export function listen(note: string, l: NoteListener): () => void {
  const set = listeners.get(note) ?? new Set();
  set.add(l);
  listeners.set(note, set);
  return () => {
    set.delete(l);
    if (!set.size) listeners.delete(note);
  };
}

export const noteEvents = {
  patch(note: string, p: Patch) {
    listeners.get(note)?.forEach((l) => l.patch(p));
  },
  noop(note: string, editor: string, n: number) {
    listeners.get(note)?.forEach((l) => l.noop(editor, n));
  },
  /** The event stream was interrupted: every editor fetches its text again. */
  reopenAll() {
    listeners.forEach((set) => set.forEach((l) => l.reopen()));
  },
};

const later = (f: () => void, ms: number) => void setTimeout(f, ms);

// --- The daemon ---

export const liveNotes: NotesBackend = {
  list: () => daemon.notes(),
  create: (title, notebook) => daemon.noteCreate(title, notebook),
  wire: (note) => ({
    open: (editor) => daemon.noteOpen(note, editor),
    edit: (edit) => daemon.noteEdit(note, edit),
    later,
  }),
  meta: (note, change) => daemon.noteMeta(note, change),
  history: (note) => daemon.noteHistory(note),
  at: (note, r) => daemon.noteAt(note, r),
  restore: (note, r) => daemon.noteRestore(note, r),
};

// --- The browser preview ---

interface PreviewNote {
  summary: NoteSummary;
  fake: FakeNote;
}

const preview = new Map<string, PreviewNote>();
let previewSeq = 100;

function previewNote(id: string, title: string, notebook: string, pinned = false): PreviewNote {
  const note: PreviewNote = {
    summary: { id, title, notebook, pinned, trashed: false, created: Date.now(), updated: Date.now(), updatedBy: mock.ME, preview: "", chars: 0 },
    fake: new FakeNote(
      (p) => {
        touch(note);
        noteEvents.patch(id, p);
      },
      (editor, n) => noteEvents.noop(id, editor, n),
    ),
  };
  preview.set(id, note);
  return note;
}

function touch(note: PreviewNote) {
  const text = note.fake.text;
  note.summary.preview = text.split(/\s+/).join(" ").trim().slice(0, 160);
  note.summary.chars = [...text].length;
  note.summary.updated = Date.now();
  useNotes.getState().changed();
}

/** Write `text` at the end of a preview note as `device`. */
function previewType(note: PreviewNote, device: string, text: string) {
  note.fake.remote(device, splice(note.fake.text.length, 0, text));
}

function seedPreview() {
  if (preview.size) return;
  const pad = previewNote(SCRATCHPAD, "", "");
  previewType(pad, mock.ME, "Call back about the lease renewal\n");
  previewType(pad, "d-mbp", "Wi-Fi at the office: ask Sam\n");

  const trip = previewNote("d-mbp:12", "Lisbon trip", "Personal", true);
  previewType(trip, "d-mbp", "# Lisbon trip\n\nFlight **AC 837**, seat 14A. Hotel near Alfama.\n\n## Packing\n\n");
  previewType(trip, mock.ME, "- [x] passport\n- [ ] adapter (type F)\n- [ ] ");
  previewType(trip, "d-nuc", "camera charger\n");
  previewType(trip, "d-mbp", "\n> Pastéis de Belém opens at 8.\n");

  const sync = previewNote("d-studio:40", "Sync design notes", "Tether");
  previewType(sync, mock.ME, "## Why Fugue\n\nRGA interleaves text typed *backwards* at the same spot; Fugue does not. See https://arxiv.org/abs/2305.00583\n\n```rust\nfn place(&self, at: usize) -> (Option<CharId>, Side)\n```\n");
  previewType(sync, "d-nuc", "\nTodo: measure keystroke latency on the laptop.\n");

  const ideas = previewNote("d-nuc:7", "Weekend", "Personal");
  previewType(ideas, "d-nuc", "Fix the bike brakes. Try the new ramen place on Queen St.");
}

/** Now and then, another device types into a preview note, so merging can be seen. */
function startPreviewPeer() {
  const words = ["also ", "bring ", "the ", "spare ", "keys ", "and ", "sunscreen. "];
  let k = 0;
  setInterval(() => {
    const trip = preview.get("d-mbp:12");
    if (trip && document.visibilityState === "visible") previewType(trip, "d-mbp", words[k++ % words.length]);
  }, 9000);
}

export const previewNotes: NotesBackend = {
  async list() {
    seedPreview();
    return [...preview.values()].map((n) => ({ ...n.summary }));
  },
  async create(title, notebook) {
    const id = `${mock.ME}:${++previewSeq}`;
    previewNote(id, title, notebook);
    useNotes.getState().changed();
    return id;
  },
  wire: (id) => ({
    open: async (editor) => {
      seedPreview();
      const note = preview.get(id);
      if (!note) throw new Error("That note does not exist");
      return note.fake.open(editor);
    },
    edit: async (edit: EditRequest): Promise<EditResult> => {
      const note = preview.get(id);
      if (!note) return { result: "refused", reason: "That note does not exist" };
      return note.fake.edit(mock.ME, edit);
    },
    later,
  }),
  async meta(id, change) {
    const note = preview.get(id);
    if (!note) return;
    Object.assign(note.summary, change, { updated: Date.now() });
    useNotes.getState().changed();
  },
  async history(id) {
    const note = preview.get(id);
    return [...(note?.fake.revisions ?? [])]
      .reverse()
      .map((r) => ({ author: r.author, first: r.first, at: r.at, counter: 0, edits: r.edits, inserted: r.inserted, deleted: r.deleted }));
  },
  async at(id, r) {
    const found = preview.get(id)?.fake.revisions.find((x) => x.at === r.at && x.author === r.author);
    if (!found) throw new Error("That version does not exist");
    return { version: 0, text: found.text, authors: found.authors.map((s) => ({ ...s })), answered: 0 };
  },
  async restore(id, r) {
    const note = preview.get(id);
    const found = note?.fake.revisions.find((x) => x.at === r.at && x.author === r.author);
    if (!note || !found) return;
    note.fake.remote(mock.ME, splice(0, note.fake.text.length, found.text) as Delta);
  },
};

// --- The list ---

interface NotesState {
  backend: NotesBackend;
  list: NoteSummary[];
  loaded: boolean;
  error: string | null;
  refresh: () => Promise<void>;
  /** Something changed: fetch the list again soon (once for a burst). */
  changed: () => void;
}

let pending: ReturnType<typeof setTimeout> | null = null;

export const useNotes = create<NotesState>((set, get) => ({
  backend: inDesktopApp ? liveNotes : previewNotes,
  list: [],
  loaded: false,
  error: null,
  async refresh() {
    try {
      set({ list: await get().backend.list(), loaded: true, error: null });
    } catch (e) {
      set({ error: typeof e === "string" ? e : e instanceof Error ? e.message : "Could not load notes", loaded: true });
    }
  },
  changed() {
    if (pending) return;
    pending = setTimeout(() => {
      pending = null;
      void get().refresh();
    }, 400);
  },
}));

if (!inDesktopApp && typeof window !== "undefined" && !new URLSearchParams(location.search).has("scratchpad")) startPreviewPeer();

/** The scratchpad window's way to its note: it may reach no other. */
export function scratchpadWire(): Wire {
  if (!inDesktopApp) return previewNotes.wire(SCRATCHPAD);
  return { open: (editor) => daemon.scratchOpen(editor), edit: (edit) => daemon.scratchEdit(edit), later };
}

/** A note's start without the Markdown marks, for the list. */
export function plainPreview(preview: string): string {
  return preview
    .replace(/(^|\s)(#{1,6}|>|[-*+]|\d+[.)])\s+(\[[ xX]\]\s+)?/g, "$1")
    .replace(/(\*\*|__|~~|`)/g, "")
    .trim();
}

/** A note's title as shown: its own, or its first line. */
export function titleOf(n: NoteSummary): string {
  if (n.id === SCRATCHPAD) return "Scratchpad";
  return n.title.trim() || plainPreview(n.preview).slice(0, 60) || "Untitled note";
}
