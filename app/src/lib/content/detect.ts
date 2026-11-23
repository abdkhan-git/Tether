// Decides what a piece of clipboard text *is*, so the UI can show it the way
// that kind of thing should be shown. Pure and synchronous; results are cached.

import { bytes } from "@/lib/format";
import { detectLanguage, languageName, type Language } from "./language";
import {
  detectSecret,
  parseColor,
  parseJwt,
  parseLink,
  parseMath,
  parsePath,
  parseTable,
  parseTime,
  textStats,
  type JwtInfo,
  type SecretKind,
  type LinkInfo,
  type PathInfo,
  type Rgba,
  type TableInfo,
  type TextStats,
  type TimeInfo,
} from "./parsers";

interface Base {
  /** Short name for lists and chips: "JSON", "Rust", "Link". */
  label: string;
  /** One line of useful facts: "Object · 12 keys · 2.4 KB". */
  summary: string;
  /** Should never leave this device. */
  sensitive: boolean;
}

export type Content = Base &
  (
    | { kind: "json"; value: unknown; shape: "object" | "array"; entries: number; depth: number }
    | { kind: "code"; language: Language; lines: number }
    | { kind: "link"; link: LinkInfo }
    | { kind: "color"; color: Rgba }
    | { kind: "jwt"; jwt: JwtInfo }
    | { kind: "secret"; secret: SecretKind }
    | { kind: "time"; time: TimeInfo }
    | { kind: "table"; table: TableInfo }
    | { kind: "path"; path: PathInfo }
    | { kind: "email"; address: string }
    | { kind: "math"; expression: string; result: number }
    | { kind: "uuid"; version: number }
    | { kind: "text"; stats: TextStats }
  );

export type ContentKind = Content["kind"];

function depthOf(v: unknown): number {
  if (v === null || typeof v !== "object") return 0;
  const children = Array.isArray(v) ? v : Object.values(v);
  return 1 + children.reduce<number>((m, c) => Math.max(m, depthOf(c)), 0);
}

const plural = (n: number, word: string) => `${n.toLocaleString()} ${word}${n === 1 ? "" : "s"}`;

export function detect(raw: string): Content {
  const text = raw.trim();
  const size = bytes(new Blob([raw]).size);
  const single = !text.includes("\n");

  const jwt = single ? parseJwt(text) : null;
  if (jwt) {
    const exp = jwt.expires ? (jwt.expires.getTime() < Date.now() ? "expired" : "valid") : "no expiry";
    return { kind: "jwt", jwt, label: "JWT", summary: `${String(jwt.header.alg)} · ${exp}`, sensitive: true };
  }

  const secret = detectSecret(text);
  if (secret && (secret.label !== "Password" || single)) {
    return { kind: "secret", secret, label: secret.label, summary: "Kept on this device", sensitive: true };
  }

  if (/^[[{]/.test(text) && text.length > 2) {
    try {
      const value = JSON.parse(text);
      if (value && typeof value === "object") {
        const shape = Array.isArray(value) ? "array" : "object";
        const entries = Array.isArray(value) ? value.length : Object.keys(value).length;
        const summary = `${shape === "array" ? "Array" : "Object"} · ${plural(entries, shape === "array" ? "item" : "key")} · depth ${depthOf(value)} · ${size}`;
        return { kind: "json", value, shape, entries, depth: depthOf(value), label: "JSON", summary, sensitive: false };
      }
    } catch {
      /* not JSON; fall through */
    }
  }

  if (single) {
    const link = parseLink(text);
    if (link) {
      const host = link.url.hostname.replace(/^www\./, "");
      const extra = link.tracking.length ? ` · ${plural(link.tracking.length, "tracker")}` : "";
      return { kind: "link", link, label: "Link", summary: `${link.title ?? host}${extra}`, sensitive: false };
    }
    if (/^[^\s@]+@[^\s@]+\.[a-z]{2,}$/i.test(text)) return { kind: "email", address: text, label: "Email", summary: text.split("@")[1], sensitive: false };
    const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-([1-8])[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.exec(text);
    if (uuid) return { kind: "uuid", version: +uuid[1], label: "UUID", summary: `Version ${uuid[1]}`, sensitive: false };
    const color = parseColor(text);
    if (color) return { kind: "color", color, label: "Colour", summary: text, sensitive: false };
    const time = parseTime(text);
    if (time) return { kind: "time", time, label: "Date & time", summary: time.date.toLocaleString(), sensitive: false };
    const path = parsePath(text);
    if (path) return { kind: "path", path, label: path.extension ? `.${path.extension} file` : "Path", summary: path.name, sensitive: false };
    const math = parseMath(text);
    if (math) return { kind: "math", ...math, label: "Calculation", summary: `= ${formatNumber(math.result)}`, sensitive: false };
  }

  const table = parseTable(text);
  if (table) {
    const cols = table.rows[0].length;
    return { kind: "table", table, label: "Table", summary: `${plural(table.rows.length - 1, "row")} × ${plural(cols, "column")}`, sensitive: false };
  }

  const language = detectLanguage(text);
  if (language) {
    const lines = text.split("\n").length;
    return { kind: "code", language, lines, label: languageName[language], summary: `${plural(lines, "line")} · ${size}`, sensitive: false };
  }

  const stats = textStats(text);
  const minutes = Math.max(1, Math.round(stats.words / 230));
  const summary = stats.words > 120 ? `${plural(stats.words, "word")} · ${minutes} min read` : plural(stats.words, "word");
  return { kind: "text", stats, label: "Text", summary, sensitive: false };
}

export function formatNumber(n: number): string {
  if (Number.isInteger(n)) return n.toLocaleString();
  return n.toLocaleString(undefined, { maximumFractionDigits: 10 });
}

const cache = new Map<string, Content>();

/** `detect`, remembered: lists call this for every row on every render. */
export function contentOf(text: string): Content {
  let hit = cache.get(text);
  if (!hit) {
    hit = detect(text);
    if (cache.size > 500) cache.delete(cache.keys().next().value!);
    cache.set(text, hit);
  }
  return hit;
}
