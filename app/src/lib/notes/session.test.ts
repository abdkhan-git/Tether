import { describe, expect, it } from "vitest";
import { apply, movePosition, type Delta } from "./delta";
import { randomDelta, rng } from "./test-util";
import { FakeNote } from "./fake-daemon";
import { applyToSpans, NoteSession, visible, type EditRequest, type EditResult, type OpenedNote, type Span } from "./session";

const ME = "me";
const PEER = "peer";

/** A network with a clock of its own: everything is delivered when the test says. */
class World {
  time = 0;
  private timers: { at: number; f: () => void }[] = [];
  note: FakeNote;
  editors: Editor[] = [];
  /** Requests fail while this is set. */
  down = false;
  /** Requests reach the daemon, but their answers are lost. */
  deaf = false;

  constructor() {
    this.note = new FakeNote(
      (p) => this.editors.forEach((e) => e.events.push(() => e.session.patch(p))),
      (editor, n) => this.editors.forEach((e) => e.events.push(() => e.session.noop(editor, n))),
      1000,
    );
  }

  later(f: () => void, ms: number) {
    this.timers.push({ at: this.time + ms, f });
  }

  async advance(ms: number) {
    const until = this.time + ms;
    for (;;) {
      const next = this.timers.filter((t) => t.at <= until).sort((a, b) => a.at - b.at)[0];
      if (!next) break;
      this.timers.splice(this.timers.indexOf(next), 1);
      this.time = Math.max(this.time, next.at);
      next.f();
      await flush();
    }
    this.time = until;
  }

  /** Let everything arrive until nothing is left to do. */
  async settle() {
    for (let i = 0; i < 60; i++) {
      await this.advance(5000);
      for (const e of this.editors) e.deliver();
      await flush();
    }
  }
}

async function flush() {
  for (let i = 0; i < 12; i++) await Promise.resolve();
}

class Editor {
  session: NoteSession;
  /** Events from the daemon, in order, not yet seen. */
  events: (() => void)[] = [];
  /** The textarea: text and caret. */
  shown = "";
  caret = 0;
  notices: string[] = [];

  constructor(world: World) {
    this.session = new NoteSession(
      ME,
      {
        open: async (editor): Promise<OpenedNote> => {
          if (world.down) throw new Error("down");
          return world.note.open(editor);
        },
        edit: async (e: EditRequest): Promise<EditResult> => {
          if (world.down) throw new Error("down");
          const result = world.note.edit(ME, e);
          if (world.deaf) throw new Error("the answer was lost");
          return result;
        },
        later: (f, ms) => world.later(f, ms),
      },
      {
        onText: (moved: Delta) => {
          this.shown = this.session.text;
          this.caret = movePosition(moved, this.caret);
        },
        onState: () => {},
        onNotice: (m) => this.notices.push(m),
      },
      () => world.time,
    );
    world.editors.push(this);
  }

  deliver(n = Infinity) {
    for (let i = 0; i < n && this.events.length; i++) this.events.shift()!();
  }

  type(at: number, remove: number, insert: string) {
    this.shown = this.shown.slice(0, at) + insert + this.shown.slice(at + remove);
    this.caret = at + insert.length;
    this.session.typed(this.shown, this.caret);
  }
}

const total = (spans: Span[]) => spans.reduce((n, s) => n + s.chars, 0);

describe("an editor", () => {
  it("shows typing at once and saves it in batches", async () => {
    const w = new World();
    const e = new Editor(w);
    await e.session.start();
    expect(e.session.status).toBe("saved");
    for (const [i, c] of [..."hello"].entries()) e.type(i, 0, c);
    expect(e.session.text).toBe("hello");
    expect(e.session.status).toBe("saving");
    expect(w.note.text).toBe("");
    await w.advance(150);
    expect(w.note.text).toBe("hello");
    expect(w.note.version).toBe(1001); // one edit, not five
    e.deliver();
    expect(e.session.status).toBe("saved");
    expect(e.session.settled).toBe(true);
  });

  it("merges what another device types at the same time, and keeps the caret", async () => {
    const w = new World();
    const e = new Editor(w);
    w.note.remote(PEER, ["shopping list"]);
    e.events = [];
    await e.session.start();
    e.shown = e.session.text;
    e.type(13, 0, ": milk");
    w.note.remote(PEER, ["My "]);
    e.deliver();
    expect(e.session.text).toBe("My shopping list: milk");
    expect(e.caret).toBe(22);
    await w.settle();
    expect(w.note.text).toBe("My shopping list: milk");
    expect(e.session.authors).toEqual([
      { device: PEER, chars: 16 },
      { device: ME, chars: 6 },
    ]);
  });

  it("undoes and redoes only what was typed here", async () => {
    const w = new World();
    const e = new Editor(w);
    await e.session.start();
    e.type(0, 0, "mine ");
    w.time += 5000;
    w.note.remote(PEER, ["theirs "]);
    e.deliver();
    e.type(e.session.text.length, 0, "again");
    expect(e.session.text).toBe("theirs mine again");
    e.session.undo();
    expect(e.session.text).toBe("theirs mine ");
    w.note.remote(PEER, ["! "]);
    e.deliver();
    e.session.undo();
    expect(e.session.text).toBe("! theirs ");
    expect(e.session.canUndo).toBe(false);
    e.session.redo();
    e.session.redo();
    expect(e.session.text).toBe("! theirs mine again");
    await w.settle();
    expect(w.note.text).toBe("! theirs mine again");
  });

  it("undoes a burst of typing in one go", async () => {
    const w = new World();
    const e = new Editor(w);
    await e.session.start();
    for (const [i, c] of [..."hello"].entries()) {
      e.type(i, 0, c);
      w.time += 100;
    }
    e.session.undo();
    expect(e.session.text).toBe("");
  });

  it("asks before a secret is recorded", async () => {
    const w = new World();
    const e = new Editor(w);
    await e.session.start();
    e.type(0, 0, "AKIAIOSFODNN7EXAMPLE");
    await w.advance(200);
    expect(e.session.status).toBe("secret");
    expect(e.session.secret).toBe("AWS access key");
    expect(w.note.text).toBe("");
    w.note.remote(PEER, ["still merging "]);
    e.deliver();
    e.session.answerSecret(false);
    expect(e.session.text).toBe("still merging ");
    e.shown = e.session.text;
    e.type(0, 0, "AKIAIOSFODNN7EXAMPLE ");
    await w.advance(200);
    e.session.answerSecret(true);
    await w.settle();
    expect(w.note.text).toBe("AKIAIOSFODNN7EXAMPLE still merging ");
  });

  it("carries on through a lost connection without losing or repeating anything", async () => {
    const w = new World();
    const e = new Editor(w);
    await e.session.start();
    // The edit reaches the daemon; its answer and its patch are both lost.
    e.type(0, 0, "once");
    w.deaf = true;
    await w.advance(200);
    e.events = [];
    w.deaf = false;
    w.down = true;
    e.type(4, 0, " and more");
    await w.advance(3000);
    expect(e.session.status).toBe("offline");
    w.note.remote(PEER, ["> "]);
    e.events = [];
    w.down = false;
    await w.settle();
    expect(w.note.text).toBe("> once and more");
    expect(e.session.text).toBe("> once and more");
    expect(e.session.status).toBe("saved");
  });

  it("picks up after the daemon restarts", async () => {
    const w = new World();
    const e = new Editor(w);
    await e.session.start();
    e.type(0, 0, "kept");
    w.deaf = true;
    await w.advance(200); // recorded, but the answer never came
    e.events = [];
    w.deaf = false;
    w.note.restart();
    await w.settle();
    expect(e.session.text).toBe("kept");
    expect(w.note.text).toBe("kept");
    e.shown = e.session.text;
    e.type(4, 0, "!");
    await w.settle();
    expect(w.note.text).toBe("kept!");
  });

  it("lets an input method finish before the text moves", async () => {
    const w = new World();
    const e = new Editor(w);
    await e.session.start();
    e.session.composing(true);
    e.type(0, 0, "漢");
    w.note.remote(PEER, ["peer "]);
    e.deliver();
    expect(e.session.text).toBe("漢");
    e.session.composing(false);
    expect(e.session.text).toBe("peer 漢");
    await w.settle();
    expect(w.note.text).toBe("peer 漢");
  });

  it("agrees with the note after any run of typing, lag, faults and restarts", async () => {
    for (let seed = 1; seed <= 120; seed++) {
      const w = new World();
      const r = rng(seed);
      const editors = [new Editor(w), new Editor(w)]; // two windows on one device
      for (const e of editors) await e.session.start();
      for (let step = 0; step < 80; step++) {
        const e = editors[r(2)];
        e.shown = e.session.text;
        const roll = r(100);
        if (roll < 45) {
          const d = randomDelta(r, e.session.text);
          const next = apply(d, e.session.text)!;
          e.shown = next;
          e.session.typed(next, r(next.length + 1));
        } else if (roll < 60) w.note.remote(PEER, randomDelta(r, w.note.text));
        else if (roll < 75) e.deliver(r(4));
        else if (roll < 85) await w.advance(r(400));
        else if (roll < 90) e.session.undo();
        else if (roll < 93) e.session.redo();
        else if (roll < 95) w.down = !w.down;
        else if (roll < 96) e.session.composing(r(2) === 0);
        else if (roll < 97 && seed % 3 === 0) e.events = []; // the stream fell behind:
        else if (roll < 98) void e.session.reopen();
        await flush();
      }
      w.down = false;
      for (const e of editors) e.session.composing(false);
      await w.settle();
      for (const e of editors) {
        await e.session.reopen();
        await w.settle();
        expect(e.session.text, `seed ${seed}`).toBe(w.note.text);
        expect(e.session.status, `seed ${seed}`).toBe("saved");
        expect(total(e.session.authors), `seed ${seed}`).toBe(w.note.text.length);
        expect(e.session.authors, `seed ${seed}`).toEqual(w.note.authors);
        expect(e.notices).toEqual([]);
      }
    }
  });
});

describe("hidden characters", () => {
  it("are shown, one for one, and survive typing around them", async () => {
    const w = new World();
    const e = new Editor(w);
    w.note.remote(PEER, ["a\rb\u202ec\u001b"]);
    e.events = [];
    await e.session.start();
    const shown = visible(e.session.text);
    expect(shown).toBe("a\u240db\ufffdc\u241b");
    expect(shown.length).toBe(e.session.text.length);
    e.shown = shown;
    e.type(shown.length, 0, "!");
    await w.settle();
    expect(w.note.text).toBe("a\rb\u202ec\u001b!");
  });
});

describe("authors", () => {
  it("follow the text", () => {
    const a: Span[] = [{ device: "a", chars: 5 }];
    const both = applyToSpans(a, [2, "xyz", -1], [{ device: "b", chars: 3 }]);
    expect(both).toEqual([
      { device: "a", chars: 2 },
      { device: "b", chars: 3 },
      { device: "a", chars: 2 },
    ]);
    expect(applyToSpans(both, [2, -3], [])).toEqual([{ device: "a", chars: 4 }]);
  });
});

