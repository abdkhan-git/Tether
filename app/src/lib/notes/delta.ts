// Text changes by position. Mirrors crates/tether-core/src/delta.rs, and is
// checked against the same generated cases (delta.test.ts).
//
// A delta walks the text: a positive number keeps that many characters, a
// negative one deletes them, a string is inserted. Whatever it does not reach
// is kept. Built with these functions it is always in its simplest form, with
// an insertion before a deletion at the same place.
//
// Inside the editor, lengths count UTF-16 units, as a textarea does. The daemon
// counts Unicode characters; `toChars` and `toUnits` convert, given the text a
// delta applies to.

export type Step = number | string;
export type Delta = readonly Step[];

const len = (s: string) => s.length;

/** Add a step, keeping the simplest form. */
function push(out: Step[], step: Step): void {
  if (step === 0 || step === "") return;
  const last = out[out.length - 1];
  if (typeof step === "number") {
    if (typeof last === "number" && Math.sign(last) === Math.sign(step)) out[out.length - 1] = last + step;
    else out.push(step);
  } else if (typeof last === "string") {
    out[out.length - 1] = last + step;
  } else if (typeof last === "number" && last < 0) {
    // Insertions go before deletions at the same place.
    const before = out[out.length - 2];
    if (typeof before === "string") out[out.length - 2] = before + step;
    else out.splice(out.length - 1, 0, step);
  } else {
    out.push(step);
  }
}

function finish(out: Step[]): Delta {
  const last = out[out.length - 1];
  if (typeof last === "number" && last > 0) out.pop();
  return out;
}

export function fromSteps(steps: Iterable<Step>): Delta {
  const out: Step[] = [];
  for (const s of steps) push(out, s);
  return finish(out);
}

/** Replace `remove` characters at `at` with `insert`. */
export function splice(at: number, remove: number, insert: string): Delta {
  return fromSteps([at, insert, -remove]);
}

export const isEmpty = (d: Delta) => d.length === 0;

/** The changed text, or null if the delta reaches past the end of it. */
export function apply(d: Delta, text: string): string | null {
  let out = "";
  let at = 0;
  for (const s of d) {
    if (typeof s === "string") {
      out += s;
      continue;
    }
    const n = Math.abs(s);
    if (at + n > text.length) return null;
    if (s > 0) out += text.slice(at, at + n);
    at += n;
  }
  return out + text.slice(at);
}

/** Walks a delta a part at a time, with the endless keep after its last step. */
class Walk {
  private i = 0;
  private used = 0;
  constructor(private readonly d: Delta) {}

  /** The current step, less what is used; undefined at the end. */
  get head(): Step | undefined {
    const s = this.d[this.i];
    return typeof s === "number" ? (s > 0 ? s - this.used : s + this.used) : s;
  }

  take(n: number): void {
    const s = this.d[this.i];
    if (s === undefined) return;
    this.used += n;
    if (typeof s === "string" || this.used >= Math.abs(s)) {
      this.i++;
      this.used = 0;
    }
  }
}

const count = (h: number | undefined) => (h === undefined ? Infinity : Math.abs(h));

/**
 * Carry `edit` and `applied`, two changes to the same text, over each other:
 * `[edit after applied, applied after edit]`. Where both insert at the same
 * place, `applied`'s text comes first.
 */
export function transform(edit: Delta, applied: Delta): [Delta, Delta] {
  const e = new Walk(edit);
  const a = new Walk(applied);
  const e2: Step[] = [];
  const a2: Step[] = [];
  for (;;) {
    const eh = e.head;
    const ah = a.head;
    if (typeof ah === "string") {
      push(e2, len(ah));
      push(a2, ah);
      a.take(0);
    } else if (typeof eh === "string") {
      push(e2, eh);
      push(a2, len(eh));
      e.take(0);
    } else if (eh === undefined && ah === undefined) {
      return [finish(e2), finish(a2)];
    } else {
      const n = Math.min(count(eh), count(ah));
      const eDeletes = eh !== undefined && eh < 0;
      const aDeletes = ah !== undefined && ah < 0;
      if (eDeletes && !aDeletes) push(e2, -n);
      else if (aDeletes && !eDeletes) push(a2, -n);
      else if (!eDeletes && !aDeletes) {
        push(e2, n);
        push(a2, n);
      }
      e.take(n);
      a.take(n);
    }
  }
}

/** One delta that does `first`, then `second`. */
export function compose(first: Delta, second: Delta): Delta {
  const a = new Walk(first);
  const b = new Walk(second);
  const out: Step[] = [];
  // How much of a string in `first` the second delta has passed.
  let within = 0;
  for (;;) {
    const ah = a.head;
    const bh = b.head;
    if (typeof bh === "string") {
      push(out, bh);
      b.take(0);
    } else if (typeof ah === "number" && ah < 0) {
      push(out, ah);
      a.take(-ah);
    } else if (ah === undefined && bh === undefined) {
      return finish(out);
    } else {
      const left = typeof ah === "string" ? ah.length - within : count(ah);
      const n = Math.min(left, count(bh));
      const deletes = bh !== undefined && bh < 0;
      if (typeof ah === "string") {
        if (n === Infinity || n >= left) {
          if (!deletes) push(out, ah.slice(within));
          within = 0;
          a.take(0);
        } else {
          if (!deletes) push(out, ah.slice(within, within + n));
          within += n;
        }
        b.take(Math.min(n, left));
      } else {
        push(out, deletes ? -n : n);
        a.take(n);
        b.take(n);
      }
    }
  }
}

/** The delta that undoes `d`, made to `text`. */
export function invert(d: Delta, text: string): Delta {
  const out: Step[] = [];
  let at = 0;
  for (const s of d) {
    if (typeof s === "string") push(out, -s.length);
    else if (s > 0) {
      push(out, s);
      at += s;
    } else {
      push(out, text.slice(at, at - s));
      at -= s;
    }
  }
  return finish(out);
}

/** Where position `at` ends up. Text inserted exactly there moves it only if `after`. */
export function movePosition(d: Delta, at: number, after = false): number {
  let old = 0;
  let shift = 0;
  for (const s of d) {
    if (old > at) break;
    if (typeof s === "string") {
      if (old < at || after) shift += s.length;
    } else if (s > 0) {
      old += s;
    } else {
      shift -= Math.min(old - s, at) - old;
      old -= s;
    }
  }
  return at + shift;
}

const isHigh = (c: number) => c >= 0xd800 && c <= 0xdbff;
const isLow = (c: number) => c >= 0xdc00 && c <= 0xdfff;

/** Would a cut at `i` split a surrogate pair? */
export function splitsPair(s: string, i: number): boolean {
  return i > 0 && i < s.length && isHigh(s.charCodeAt(i - 1)) && isLow(s.charCodeAt(i));
}

/**
 * What turned `before` into `after`. `caret` (where the caret ended up) decides
 * between equal readings: typing a second "l" into "hel|o" is an insertion at
 * the caret, not somewhere else in the run of l's.
 */
export function diff(before: string, after: string, caret = after.length): Delta {
  const most = Math.min(before.length, after.length);
  let tail = 0;
  const maxTail = Math.min(most, Math.max(0, after.length - caret));
  while (tail < maxTail && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail++;
  let head = 0;
  while (head < most - tail && before[head] === after[head]) head++;
  // The caret is only a hint: if the text after it changed too, match on.
  while (tail < most - head && before[before.length - 1 - tail] === after[after.length - 1 - tail]) tail++;
  if (splitsPair(before, head) || splitsPair(after, head)) head--;
  if (splitsPair(before, before.length - tail) || splitsPair(after, after.length - tail)) tail--;
  return splice(head, before.length - tail - head, after.slice(head, after.length - tail));
}

const points = (s: string) => {
  let n = s.length;
  for (let i = 1; i < s.length; i++) if (splitsPair(s, i)) n--;
  return n;
};

/** UTF-16 units to Unicode characters, as the daemon counts. */
export function toChars(d: Delta, text: string): Delta {
  let at = 0;
  return d.map((s) => {
    if (typeof s === "string") return s;
    const n = points(text.slice(at, at + Math.abs(s)));
    at += Math.abs(s);
    return s > 0 ? n : -n;
  });
}

/** Unicode characters to UTF-16 units. */
export function toUnits(d: Delta, text: string): Delta {
  let at = 0;
  return d.map((s) => {
    if (typeof s === "string") return s;
    const start = at;
    for (let n = Math.abs(s); n > 0 && at < text.length; n--) at += splitsPair(text, at + 1) ? 2 : 1;
    return s > 0 ? at - start : start - at;
  });
}

/** Replace halves of surrogate pairs (they cannot be sent) with U+FFFD. */
export function wellFormed(s: string): string {
  return s.replace(/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/g, "�");
}
