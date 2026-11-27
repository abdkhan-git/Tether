// One open note, as an editor window holds it. No DOM and no network here: the
// component feeds it what the user did and what the daemon said, and shows
// `text`. Unit-tested against a stand-in daemon (session.test.ts).
//
// The daemon decides the order of changes. At any moment the editor knows:
//   base     the text at a version the daemon gave out
//   sent     one edit on its way (at most one), made against `base`
//   buffer   what has been typed since, made against `base` + `sent`
//   text     base + sent + buffer: what the user sees
// Changes from elsewhere arrive as patches to `base`. Each is carried over
// `sent` and `buffer` (and they over it) so the user's caret and typing stay
// put. The daemon answers an edit with the patch it became.

import { apply, compose, diff, invert, isEmpty, movePosition, splice, toChars, toUnits, transform, wellFormed, type Delta } from "./delta";

/** So many UTF-16 units in a row typed on one device. */
export interface Span {
  device: string;
  chars: number;
}

export interface OpenedNote {
  version: number;
  text: string;
  /** In Unicode characters. */
  authors: Span[];
  /** The last of this editor's edits the daemon has made. */
  answered: number;
}

export interface EditRequest {
  base: number;
  delta: Delta;
  editor: string;
  n: number;
  allowSecret: boolean;
}

export type EditResult = { result: "done" } | { result: "stale" } | { result: "secret"; kind: string } | { result: "refused"; reason: string };

export interface Patch {
  from: number;
  to: number;
  /** In Unicode characters. */
  delta: Delta;
  authors: Span[];
  editor: string | null;
  n: number;
}

/** What the session needs from outside: the daemon, and the clock. */
export interface Wire {
  open(editor: string): Promise<OpenedNote>;
  edit(edit: EditRequest): Promise<EditResult>;
  /** Run `f` after `ms`. */
  later(f: () => void, ms: number): void;
}

export type Status = "opening" | "saved" | "saving" | "offline" | "secret" | "failed";

export interface Listener {
  /** The text changed for a reason other than the user's typing. `moved` is
   * the change, to keep the caret in place. */
  onText(moved: Delta): void;
  /** Status, authors or undo changed. */
  onState(): void;
  /** Something the user should read. */
  onNotice(message: string): void;
}

/** Typing is gathered this long before it is sent. */
export const BATCH_MS = 150;
/** Wait before asking again when the daemon cannot be reached. */
const RETRY_MS = 1000;
/** Typing within this long is undone as one. */
const UNDO_MS = 700;
const MAX_UNDO = 500;

function randomId(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, "0")).join("");
}

/** Spans counted in characters, as spans counted in UTF-16 units of `text`. */
export function spansToUnits(spans: Span[], text: string): Span[] {
  let at = 0;
  return spans.map(({ device, chars }) => {
    const units = toUnits([chars], text.slice(at))[0] as number;
    at += units;
    return { device, chars: units };
  });
}

/** `spans` after `d`, whose insertions (in order) were written by `by`. */
export function applyToSpans(spans: Span[], d: Delta, by: Span[]): Span[] {
  const out: Span[] = [];
  const add = (device: string, chars: number) => {
    if (chars <= 0) return;
    const last = out[out.length - 1];
    if (last && last.device === device) last.chars += chars;
    else out.push({ device, chars });
  };
  const queue = spans.map((s) => ({ ...s }));
  const take = (from: Span[], n: number, each: (device: string, chars: number) => void) => {
    while (n > 0 && from.length) {
      const k = Math.min(n, from[0].chars);
      each(from[0].device, k);
      from[0].chars -= k;
      n -= k;
      if (from[0].chars === 0) from.shift();
    }
  };
  const writers = by.map((s) => ({ ...s }));
  for (const step of d) {
    if (typeof step === "string") take(writers, step.length, add);
    else if (step > 0) take(queue, step, add);
    else take(queue, -step, () => {});
  }
  queue.forEach((s) => add(s.device, s.chars));
  return out;
}

export class NoteSession {
  readonly editor = randomId();
  text = "";
  /** Who wrote `text`, in UTF-16 units. */
  authors: Span[] = [];
  status: Status = "opening";
  /** Why the note could not be opened, while it cannot. */
  error: string | null = null;
  /** The kind of secret the user is being asked about. */
  secret: string | null = null;

  private base = "";
  private version = -1;
  private sent: Delta | null = null;
  /** The request for `sent` is out. */
  private asking = false;
  private buffer: Delta = [];
  private n = 0;
  private batch = false;
  private closed = false;
  private opening = false;
  private allowSecrets = false;
  private undos: { delta: Delta; at: number }[] = [];
  private redos: Delta[] = [];
  /** While an input method is composing, the text must not move under it. */
  private held: (() => void)[] | null = null;
  /** Patches that came while the note was being opened. */
  private early: Patch[] = [];

  constructor(
    private readonly me: string,
    private readonly wire: Wire,
    private readonly listener: Listener,
    private readonly now: () => number = () => Date.now(),
  ) {}

  get canUndo() {
    return this.undos.length > 0;
  }
  get canRedo() {
    return this.redos.length > 0;
  }
  /** Everything typed has been recorded. */
  get settled() {
    return !this.sent && isEmpty(this.buffer);
  }

  async start(): Promise<void> {
    await this.reopen();
  }

  /** Stop. Whatever is still unsent is sent first. */
  close(): void {
    this.flush();
    this.closed = true;
  }

  /**
   * Fetch the text again and carry on: at the start, and whenever patches may
   * have been missed (the daemon restarted, or the stream fell behind).
   */
  async reopen(): Promise<void> {
    if (this.closed || this.opening) return;
    this.opening = true;
    this.early = [];
    try {
      const opened = await this.wire.open(this.editor);
      if (this.closed) return;
      this.opening = false;
      this.error = null;
      const early = this.early;
      this.early = [];
      this.hold(() => {
        this.resume(opened);
        early.forEach((p) => this.patch(p));
        this.pump();
      });
    } catch (e) {
      this.opening = false;
      this.error = typeof e === "string" ? e : e instanceof Error ? e.message : "The note could not be opened";
      this.setStatus(this.version >= 0 ? "offline" : "failed");
      if (!this.closed) this.wire.later(() => void this.reopen(), RETRY_MS);
    }
  }

  private resume(opened: OpenedNote): void {
    const first = this.version < 0;
    if (this.sent) {
      // Did the daemon make the edit that was on its way? It says so; after a
      // restart it no longer knows, but then the text itself may show it.
      const made = apply(this.sent, this.base) ?? this.base;
      if (opened.answered >= this.n || (opened.answered === 0 && opened.text === made)) {
        this.base = made;
        this.sent = null;
      }
    }
    // Treat what changed meanwhile as one change from elsewhere.
    const missed = diff(this.base, opened.text);
    this.version = opened.version;
    this.remote(missed, null);
    this.authors = this.withPending(spansToUnits(opened.authors, opened.text));
    this.asking = false;
    this.setStatus(this.settled ? "saved" : "saving");
    if (first) this.undos = [];
    this.listener.onState();
  }

  /** Who wrote `text`, given who wrote `base`. */
  private withPending(authors: Span[]): Span[] {
    const pending = this.sent ? compose(this.sent, this.buffer) : this.buffer;
    const mine = pending.reduce<number>((n, s) => n + (typeof s === "string" ? s.length : 0), 0);
    return applyToSpans(authors, pending, [{ device: this.me, chars: mine }]);
  }

  // --- The user ---

  /** The user changed the text shown (`visible(text)`); `caret` is where the caret ended up. */
  typed(shown: string, caret: number): void {
    const clean = wellFormed(shown);
    this.local(diff(visible(this.text), clean, Math.min(caret, clean.length)), true);
  }

  /** Replace part of the text (toolbar, checkbox, list continuation). */
  replace(at: number, remove: number, insert: string): void {
    const d = splice(at, remove, wellFormed(insert));
    if (apply(d, this.text) === null || isEmpty(d)) return;
    this.local(d, false);
    this.listener.onText(d);
  }

  undo(): Delta | null {
    const entry = this.undos.pop();
    if (!entry) return null;
    this.redos.push(invert(entry.delta, this.text));
    this.make(entry.delta);
    return entry.delta;
  }

  redo(): Delta | null {
    const d = this.redos.pop();
    if (!d) return null;
    this.undos.push({ delta: invert(d, this.text), at: -Infinity });
    this.make(d);
    return d;
  }

  private local(d: Delta, typing: boolean): void {
    if (isEmpty(d)) return;
    const inverse = invert(d, this.text);
    const last = this.undos[this.undos.length - 1];
    const t = this.now();
    if (typing && last && t - last.at < UNDO_MS) {
      last.delta = compose(inverse, last.delta);
      last.at = t;
    } else {
      this.undos.push({ delta: inverse, at: typing ? t : -Infinity });
      if (this.undos.length > MAX_UNDO) this.undos.shift();
    }
    this.redos = [];
    this.make(d);
  }

  private make(d: Delta): void {
    const text = apply(d, this.text);
    if (text === null) return;
    const mine = d.reduce<number>((n, s) => n + (typeof s === "string" ? s.length : 0), 0);
    this.authors = applyToSpans(this.authors, d, [{ device: this.me, chars: mine }]);
    this.text = text;
    this.buffer = compose(this.buffer, d);
    if (this.status === "saved") this.setStatus("saving");
    if (!this.batch) {
      this.batch = true;
      this.wire.later(() => {
        this.batch = false;
        this.pump();
      }, BATCH_MS);
    }
    this.listener.onState();
  }

  /** Send what has been typed now, without waiting for the batch. */
  flush(): void {
    this.pump();
  }

  // --- Sending ---

  private pump(): void {
    if (this.closed || this.asking || this.opening || this.version < 0 || this.status === "secret") return;
    if (!this.sent) {
      if (isEmpty(this.buffer)) return;
      this.sent = this.buffer;
      this.buffer = [];
      this.n++;
    }
    void this.ask();
  }

  private async ask(): Promise<void> {
    if (!this.sent) return;
    this.asking = true;
    const n = this.n;
    const request: EditRequest = {
      base: this.version,
      delta: toChars(this.sent, this.base),
      editor: this.editor,
      n,
      allowSecret: this.allowSecrets,
    };
    let result: EditResult;
    try {
      result = await this.wire.edit(request);
    } catch {
      if (this.closed || n !== this.n) return;
      this.asking = false;
      this.setStatus("offline");
      // The daemon may have made it: opening the note again says.
      this.wire.later(() => void this.reopen(), RETRY_MS);
      return;
    }
    if (this.closed) return;
    switch (result.result) {
      case "done":
        // The patch is the answer (it may be here already, or on its way).
        if (n === this.n && this.sent) this.wire.later(() => this.overdue(n), 4 * RETRY_MS);
        return;
      case "stale":
        this.asking = false;
        void this.reopen();
        return;
      case "secret":
        this.asking = false;
        this.secret = result.kind;
        this.setStatus("secret");
        return;
      case "refused":
        this.asking = false;
        this.giveUp();
        this.listener.onNotice(result.reason);
        return;
    }
  }

  /** The patch that answers edit `n` has not come: it was missed. */
  private overdue(n: number): void {
    if (!this.closed && this.sent && this.asking && this.n === n) {
      this.asking = false;
      void this.reopen();
    }
  }

  /** The user's answer about the secret: put it in the note, or take it out. */
  answerSecret(keep: boolean): void {
    if (this.status !== "secret") return;
    this.secret = null;
    if (keep) {
      this.allowSecrets = true;
      this.setStatus("saving");
      this.pump();
    } else {
      this.giveUp();
    }
  }

  /** Put the text back to what the daemon has. */
  private giveUp(): void {
    const pending = this.sent ? compose(this.sent, this.buffer) : this.buffer;
    const back = invert(pending, this.base);
    this.authors = applyToSpans(this.authors, back, this.lost(pending));
    this.text = this.base;
    this.sent = null;
    this.buffer = [];
    this.undos = [];
    this.redos = [];
    this.setStatus("saved");
    this.listener.onText(back);
    this.listener.onState();
  }

  /** Text put back after a refusal: its authors come with the next opening. */
  private lost(pending: Delta): Span[] {
    const chars = pending.reduce<number>((n, s) => n + (typeof s === "number" && s < 0 ? -s : 0), 0);
    if (chars > 0) void this.reopen();
    return [{ device: this.me, chars }];
  }

  // --- The daemon ---

  patch(p: Patch): void {
    if (this.closed) return;
    if (this.opening) {
      this.early.push(p);
      return;
    }
    this.hold(() => {
      if (this.version < 0 || p.to <= this.version) return; // from before the note was opened
      if (p.from !== this.version) {
        void this.reopen();
        return;
      }
      const delta = toUnits(p.delta, this.base);
      this.version = p.to;
      if (p.editor === this.editor && p.n === this.n && this.sent) {
        // Our own edit, as recorded. The text already shows it.
        const next = apply(delta, this.base);
        const expected = apply(this.sent, this.base);
        this.sent = null;
        this.asking = false;
        if (next === null || next !== expected) {
          // Cannot happen unless something is wrong: show the daemon's text.
          void this.reopen();
          return;
        }
        this.base = next;
        this.answered();
      } else {
        this.remote(delta, p.authors);
      }
    });
  }

  /** The daemon recorded our edit, and it changed nothing. */
  noop(editor: string, n: number): void {
    if (editor !== this.editor || n !== this.n || !this.sent || this.closed) return;
    this.hold(() => {
      if (!this.sent) return;
      const gone = this.sent;
      this.sent = null;
      this.asking = false;
      if (!isEmpty(gone)) {
        // It said so about an edit that does change our text: start over.
        void this.reopen();
        return;
      }
      this.answered();
    });
  }

  private answered(): void {
    if (this.settled) this.setStatus("saved");
    else if (!this.batch) this.pump();
    this.listener.onState();
  }

  /** A change from elsewhere to `base`. */
  private remote(delta: Delta, authors: Span[] | null): void {
    if (isEmpty(delta)) return;
    const next = apply(delta, this.base);
    if (next === null) {
      void this.reopen();
      return;
    }
    const by = authors ? spansToUnits(authors, delta.filter((s) => typeof s === "string").join("")) : null;
    this.base = next;
    let seen = delta;
    if (this.sent) [this.sent, seen] = transform(this.sent, seen);
    [this.buffer, seen] = transform(this.buffer, seen);
    const text = apply(seen, this.text);
    if (text === null) {
      void this.reopen();
      return;
    }
    this.text = text;
    if (by) this.authors = applyToSpans(this.authors, seen, by);
    // What undo would do, carried over this change too.
    let over = seen;
    for (let i = this.undos.length - 1; i >= 0; i--) {
      [this.undos[i].delta, over] = transform(this.undos[i].delta, over);
    }
    over = seen;
    for (let i = this.redos.length - 1; i >= 0; i--) {
      [this.redos[i], over] = transform(this.redos[i], over);
    }
    this.undos = this.undos.filter((u) => !isEmpty(u.delta));
    this.redos = this.redos.filter((d) => !isEmpty(d));
    this.listener.onText(seen);
    this.listener.onState();
  }

  // --- Input methods ---

  /** An input method started composing: changes from elsewhere wait. */
  composing(on: boolean): void {
    if (on) {
      this.held ??= [];
      return;
    }
    const held = this.held ?? [];
    this.held = null;
    held.forEach((f) => f());
  }

  private hold(f: () => void): void {
    if (this.held) this.held.push(f);
    else f();
  }

  private setStatus(status: Status): void {
    if (this.status === status) return;
    this.status = status;
    this.listener.onState();
  }
}

/**
 * The text as an editor shows it. A textarea rewrites some characters (a lone
 * carriage return becomes a newline) and hides others (escape codes, direction
 * overrides that make text read differently from how it is stored). Each is
 * shown as a visible stand-in of the same length, so what the user sees is
 * what is stored and positions line up one to one.
 */
export function visible(text: string): string {
  // eslint-disable-next-line no-control-regex
  return text.replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f\u200e\u200f\u202a-\u202e\u2066-\u2069]/g, (c) => {
    const code = c.charCodeAt(0);
    if (code < 0x20) return String.fromCharCode(0x2400 + code);
    if (code === 0x7f) return "\u2421";
    return "\ufffd";
  });
}

/** Where the caret (or the end of a selection) goes when text moves around it. */
export function moveCaret(d: Delta, at: number): number {
  return movePosition(d, at);
}
