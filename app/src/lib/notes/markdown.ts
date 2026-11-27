// A small Markdown reader for notes: headings, paragraphs, lists, checklists,
// quotes, rules, fenced code, and inline code, bold, italic, strikethrough and
// links. It returns plain data; the page renders it with React, which escapes
// everything (no HTML from the note is ever inserted). Notes come from other
// devices, so every loop is bounded: inline markup is read only in the first
// MAX_INLINE characters of a line, nesting stops at MAX_DEPTH, and nothing
// backtracks.

export type Inline =
  | { t: "text"; v: string }
  | { t: "code"; v: string }
  | { t: "strong" | "em" | "del"; c: Inline[] }
  | { t: "link"; href: string; c: Inline[] };

export interface ListItem {
  /** Indentation level, from 0. */
  depth: number;
  /** For an ordered list, the number written. */
  number: number | null;
  /** Checklist state, or null for a plain item. */
  checked: boolean | null;
  /** Where the box's "[" is in the note (UTF-16 offset), to tick it. */
  box: number;
  inline: Inline[];
}

export type Block =
  | { kind: "heading"; level: number; inline: Inline[] }
  | { kind: "para"; inline: Inline[] }
  | { kind: "list"; items: ListItem[] }
  | { kind: "code"; lang: string; text: string }
  | { kind: "quote"; inline: Inline[] }
  | { kind: "rule" };

const MAX_INLINE = 4000;
const MAX_DEPTH = 4;
const MAX_LIST_DEPTH = 8;

const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const FENCE = /^\s{0,3}(`{3,}|~{3,})\s*([\w+#.-]*)/;
const ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(\[([ xX])\]\s+)?(.*)$/;

export function parse(text: string): Block[] {
  const lines = text.split("\n");
  const starts: number[] = [];
  let offset = 0;
  for (const line of lines) {
    starts.push(offset);
    offset += line.length + 1;
  }
  const out: Block[] = [];
  let para: string[] = [];
  const endPara = () => {
    if (para.length) out.push({ kind: "para", inline: inline(para.join("\n")) });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fence = FENCE.exec(line);
    if (fence) {
      endPara();
      const marker = fence[1];
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith(marker)) body.push(lines[i++]);
      out.push({ kind: "code", lang: fence[2].toLowerCase(), text: body.join("\n") });
      continue;
    }
    if (!line.trim()) {
      endPara();
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      endPara();
      out.push({ kind: "heading", level: heading[1].length, inline: inline(heading[2]) });
      continue;
    }
    if (RULE.test(line)) {
      endPara();
      out.push({ kind: "rule" });
      continue;
    }
    if (line.startsWith(">")) {
      endPara();
      const quoted: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) quoted.push(lines[i++].replace(/^>\s?/, ""));
      i--;
      out.push({ kind: "quote", inline: inline(quoted.join("\n")) });
      continue;
    }
    const item = ITEM.exec(line);
    if (item) {
      endPara();
      const items: ListItem[] = [];
      while (i < lines.length) {
        const m = ITEM.exec(lines[i]);
        if (!m) break;
        const indent = m[1].replace(/\t/g, "  ").length;
        const marker = m[2];
        const box = m[3] ? starts[i] + m[1].length + marker.length + lines[i].slice(m[1].length + marker.length).indexOf("[") : -1;
        items.push({
          depth: Math.min(MAX_LIST_DEPTH, Math.floor(indent / 2)),
          number: /\d/.test(marker) ? Number.parseInt(marker, 10) : null,
          checked: m[4] === undefined ? null : m[4] !== " ",
          box,
          inline: inline(m[5]),
        });
        i++;
      }
      i--;
      out.push({ kind: "list", items });
      continue;
    }
    para.push(line);
  }
  endPara();
  return out;
}

/** Links the page may open: the app checks again before opening anything. */
function safeHref(href: string): string | null {
  return /^(https?:\/\/|mailto:)/i.test(href.trim()) ? href.trim() : null;
}

const BARE_URL = /^https?:\/\/[^\s<>"'`]+/i;
const PAIRS: [string, "strong" | "em" | "del"][] = [
  ["**", "strong"],
  ["__", "strong"],
  ["~~", "del"],
  ["*", "em"],
  ["_", "em"],
];

export function inline(text: string, depth = 0): Inline[] {
  const out: Inline[] = [];
  const head = text.length > MAX_INLINE ? text.slice(0, MAX_INLINE) : text;
  const rest = text.length > MAX_INLINE ? text.slice(MAX_INLINE) : "";
  let plain = "";
  const pushText = (v: string) => {
    plain += v;
  };
  const endText = () => {
    if (plain) out.push({ t: "text", v: plain });
    plain = "";
  };
  let i = 0;
  scan: while (i < head.length) {
    const c = head[i];
    if (c === "\\" && i + 1 < head.length && /[\\`*_~[\]()#>-]/.test(head[i + 1])) {
      pushText(head[i + 1]);
      i += 2;
      continue;
    }
    if (c === "`") {
      const end = head.indexOf("`", i + 1);
      if (end > i + 1) {
        endText();
        out.push({ t: "code", v: head.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    if (c === "[" && depth < MAX_DEPTH) {
      const close = head.indexOf("]", i + 1);
      if (close > i && head[close + 1] === "(") {
        const end = head.indexOf(")", close + 2);
        const href = end > 0 ? safeHref(head.slice(close + 2, end)) : null;
        if (href) {
          endText();
          out.push({ t: "link", href, c: inline(head.slice(i + 1, close), depth + 1) });
          i = end + 1;
          continue;
        }
      }
    }
    if ((c === "h" || c === "H") && (i === 0 || /\s|\(/.test(head[i - 1]))) {
      const url = BARE_URL.exec(head.slice(i, i + 2048));
      if (url) {
        const href = url[0].replace(/[.,;:!?)]+$/, "");
        endText();
        out.push({ t: "link", href, c: [{ t: "text", v: href }] });
        i += href.length;
        continue;
      }
    }
    if (depth < MAX_DEPTH) {
      for (const [mark, t] of PAIRS) {
        if (!head.startsWith(mark, i) || head[i + mark.length] === " " || head[i + mark.length] === undefined) continue;
        // `_` inside a word (snake_case) is not emphasis.
        if (mark[0] === "_" && i > 0 && /\w/.test(head[i - 1])) continue;
        const end = head.indexOf(mark, i + mark.length);
        if (end > i + mark.length && head[end - 1] !== " ") {
          endText();
          out.push({ t, c: inline(head.slice(i + mark.length, end), depth + 1) });
          i = end + mark.length;
          continue scan;
        }
      }
    }
    pushText(c);
    i++;
  }
  pushText(rest);
  endText();
  return out;
}

/** The text a checklist box at `box` should be changed to, to tick or untick it. */
export function toggled(checked: boolean): string {
  return checked ? "[ ]" : "[x]";
}

/** Characters and words, for the editor's footer. */
export function counts(text: string): { words: number; chars: number } {
  const words = text.match(/\S+/g)?.length ?? 0;
  return { words, chars: [...text].length };
}
