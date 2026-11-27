// A stand-in for the daemon's side of a note: versions, rebasing stale edits,
// one answer per edit. The browser preview runs on it (data/notes.ts), and the
// session tests drive editors against it. It keeps plain text where the real
// daemon keeps a CRDT (crates/tether-core/src/live.rs).

import { apply, isEmpty, toChars, toUnits, transform, type Delta } from "./delta";
import { applyToSpans, spansToUnits, type EditRequest, type EditResult, type OpenedNote, type Patch, type Span } from "./session";

export interface FakeRevision {
  author: string;
  first: number;
  at: number;
  text: string;
  authors: Span[];
  inserted: number;
  deleted: number;
  edits: number;
}

export class FakeNote {
  text = "";
  /** In UTF-16 units. */
  authors: Span[] = [];
  version: number;
  revisions: FakeRevision[] = [];
  private recent: { from: number; delta: Delta }[] = [];
  private answered = new Map<string, number>();

  constructor(
    private readonly emit: (p: Patch) => void,
    private readonly emitNoop: (editor: string, n: number) => void = () => {},
    start = 1,
  ) {
    this.version = start;
  }

  open(editor: string): OpenedNote {
    return { version: this.version, text: this.text, authors: this.spansInChars(), answered: this.answered.get(editor) ?? 0 };
  }

  edit(me: string, e: EditRequest): EditResult {
    if (e.n <= (this.answered.get(e.editor) ?? 0)) return { result: "done" };
    const at = e.base === this.version ? this.recent.length : this.recent.findIndex((p) => p.from === e.base);
    if (at < 0) return { result: "stale" };
    // Everything here counts characters, as the daemon does; only for the
    // final text are units needed.
    let delta = e.delta;
    for (const p of this.recent.slice(at)) [delta] = transformChars(delta, p.delta);
    if (!e.allowSecret && delta.some((s) => typeof s === "string" && /\bAKIA[A-Z0-9]{16}\b/.test(s))) {
      return { result: "secret", kind: "AWS access key" };
    }
    this.answered.set(e.editor, e.n);
    if (isEmpty(delta)) this.emitNoop(e.editor, e.n);
    else this.commit(me, delta, e.editor, e.n);
    return { result: "done" };
  }

  /** A change typed on another device. `delta` in UTF-16 units. */
  remote(device: string, delta: Delta): void {
    if (!isEmpty(delta)) this.commit(device, toChars(delta, this.text), null, 0);
  }

  /** The daemon restarted: versions start somewhere else, nothing is remembered. */
  restart(): void {
    this.version += 1_000_000;
    this.recent = [];
    this.answered.clear();
  }

  private commit(device: string, delta: Delta, editor: string | null, n: number): void {
    const units = toUnits(delta, this.text);
    const next = apply(units, this.text);
    if (next === null) throw new Error("the edit does not fit the note");
    const from = this.version++;
    this.text = next;
    const inserted = delta.reduce<number>((k, s) => k + (typeof s === "string" ? s.length : 0), 0);
    const deleted = delta.reduce<number>((k, s) => k + (typeof s === "number" && s < 0 ? -s : 0), 0);
    this.authors = applyToSpans(this.authors, units, [{ device, chars: inserted }]);
    this.recent.push({ from, delta });
    const now = Date.now();
    const last = this.revisions[this.revisions.length - 1];
    const revision = { author: device, at: now, text: this.text, authors: this.authors };
    if (last && last.author === device && now - last.at < 120_000) Object.assign(last, revision, { inserted: last.inserted + inserted, deleted: last.deleted + deleted, edits: last.edits + 1 });
    else this.revisions.push({ ...revision, first: now, inserted, deleted, edits: 1 });
    const chars = delta.reduce<number>((k, s) => k + (typeof s === "string" ? [...s].length : 0), 0);
    this.emit({ from, to: this.version, delta, authors: [{ device, chars }], editor, n });
  }

  private spansInChars(): Span[] {
    let at = 0;
    return this.authors.map(({ device, chars }) => {
      const n = [...this.text.slice(at, at + chars)].length;
      at += chars;
      return { device, chars: n };
    });
  }
}

/** `transform` on deltas that count characters: the arithmetic is the same, but
 * an insertion's length must be counted in characters too. Mapping each
 * character to one unit first keeps one implementation. */
function transformChars(edit: Delta, applied: Delta): [Delta, Delta] {
  const table: string[] = [];
  const codes = new Map<string, string>();
  const squash = (d: Delta): Delta =>
    d.map((s) =>
      typeof s === "string"
        ? [...s]
            .map((c) => {
              let code = codes.get(c);
              if (!code) {
                code = String.fromCharCode(0xe000 + table.length);
                table.push(c);
                codes.set(c, code);
              }
              return code;
            })
            .join("")
        : s,
    );
  const spread = (d: Delta): Delta => d.map((s) => (typeof s === "string" ? [...s].map((c) => table[c.charCodeAt(0) - 0xe000]).join("") : s));
  const [e, a] = transform(squash(edit), squash(applied));
  return [spread(e), spread(a)];
}

export { spansToUnits };
