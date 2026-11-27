import { describe, expect, it } from "vitest";
import { counts, inline, parse, type ListItem } from "./markdown";

describe("markdown", () => {
  it("reads the usual blocks", () => {
    const blocks = parse("# Title\n\nSome *text* here.\nStill the same paragraph.\n\n---\n> quoted\n> more\n\n```rust\nfn main() {}\n```\n");
    expect(blocks.map((b) => b.kind)).toEqual(["heading", "para", "rule", "quote", "code"]);
    expect(blocks[0]).toEqual({ kind: "heading", level: 1, inline: [{ t: "text", v: "Title" }] });
    expect(blocks[4]).toEqual({ kind: "code", lang: "rust", text: "fn main() {}" });
  });

  it("reads checklists and knows where each box is", () => {
    const text = "Groceries\n\n- [ ] milk\n- [x] bread\n  - [ ] rye\n1. first\n";
    const list = parse(text).find((b) => b.kind === "list") as { items: ListItem[] };
    expect(list.items.map((i) => [i.depth, i.checked, i.number])).toEqual([
      [0, false, null],
      [0, true, null],
      [1, false, null],
      [0, null, 1],
    ]);
    for (const item of list.items.slice(0, 3)) expect(text.slice(item.box, item.box + 3)).toMatch(/^\[[ x]\]$/);
  });

  it("reads inline markup without being fooled", () => {
    expect(inline("a **b** _c_ ~~d~~ `e*f*`")).toEqual([
      { t: "text", v: "a " },
      { t: "strong", c: [{ t: "text", v: "b" }] },
      { t: "text", v: " " },
      { t: "em", c: [{ t: "text", v: "c" }] },
      { t: "text", v: " " },
      { t: "del", c: [{ t: "text", v: "d" }] },
      { t: "text", v: " " },
      { t: "code", v: "e*f*" },
    ]);
    expect(inline("snake_case_name and 2 * 3 * 4")).toEqual([{ t: "text", v: "snake_case_name and 2 * 3 * 4" }]);
    expect(inline("\\*not emphasis\\*")).toEqual([{ t: "text", v: "*not emphasis*" }]);
  });

  it("makes links only for web and email addresses", () => {
    expect(inline("[site](https://example.com)")).toEqual([{ t: "link", href: "https://example.com", c: [{ t: "text", v: "site" }] }]);
    expect(inline("see https://example.com/a.")).toEqual([
      { t: "text", v: "see " },
      { t: "link", href: "https://example.com/a", c: [{ t: "text", v: "https://example.com/a" }] },
      { t: "text", v: "." },
    ]);
    for (const bad of ["[x](javascript:alert(1))", "[x](file:///C:/Windows)", "[x](\\\\server\\share)", "[x](ms-msdt:foo)"]) {
      expect(JSON.stringify(inline(bad))).not.toContain('"link"');
    }
  });

  it("stays fast on hostile input", () => {
    const started = performance.now();
    parse("*".repeat(200_000));
    parse("[".repeat(100_000) + "](".repeat(50_000));
    parse(`${"- ".repeat(5_000)}x\n`.repeat(20));
    parse("**".repeat(100_000));
    expect(performance.now() - started).toBeLessThan(2000);
    const deep = inline("**".repeat(30) + "x" + "**".repeat(30));
    expect(JSON.stringify(deep).length).toBeLessThan(10_000);
  });

  it("counts words and characters", () => {
    expect(counts("two words 😀")).toEqual({ words: 3, chars: 11 });
    expect(counts("")).toEqual({ words: 0, chars: 0 });
  });
});
