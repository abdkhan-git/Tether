// Small, pure parsers for the kinds of things people copy. Each returns null
// when the text is not that kind, so detect.ts can try them in order.

/* ---------- Links ---------- */

const TRACKING = /^(utm_\w+|fbclid|gclid|dclid|gbraid|wbraid|msclkid|yclid|twclid|igshid|mc_cid|mc_eid|_hsenc|_hsmi|mkt_tok|ref_src|ref_url|si|spm|li_fat_id|oly_anon_id|oly_enc_id|vero_id|s_cid)$/i;

export interface LinkInfo {
  url: URL;
  tracking: string[];
  clean: string;
  /** Recognised destination, e.g. "Pull request #412 · aurae-rs/aurae". */
  title?: string;
}

export function parseLink(text: string): LinkInfo | null {
  if (/\s/.test(text)) return null;
  const candidate = /^www\./i.test(text) ? `https://${text}` : text;
  if (!/^https?:\/\//i.test(candidate)) return null;
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    return null;
  }
  if (!url.hostname.includes(".")) return null;
  const tracking = [...url.searchParams.keys()].filter((k) => TRACKING.test(k));
  const cleanUrl = new URL(url.href);
  // YouTube's `si` is tracking, but elsewhere it can be meaningful.
  for (const k of tracking) if (k !== "si" || /youtu/.test(url.hostname)) cleanUrl.searchParams.delete(k);
  const kept = tracking.filter((k) => k !== "si" || /youtu/.test(url.hostname));
  return { url, tracking: kept, clean: cleanUrl.href, title: linkTitle(url) };
}

function linkTitle(url: URL): string | undefined {
  const host = url.hostname.replace(/^www\./, "");
  const parts = url.pathname.split("/").filter(Boolean);
  if (host === "github.com" && parts.length >= 2) {
    const repo = `${parts[0]}/${parts[1]}`;
    if (parts[2] === "pull" && parts[3]) return `Pull request #${parts[3]} · ${repo}`;
    if (parts[2] === "issues" && parts[3]) return `Issue #${parts[3]} · ${repo}`;
    if (parts[2] === "blob" && parts.length > 4) return `${parts.slice(4).join("/")} · ${repo}`;
    if (parts[2] === "commit" && parts[3]) return `Commit ${parts[3].slice(0, 7)} · ${repo}`;
    if (parts.length === 2) return `Repository · ${repo}`;
  }
  if ((host === "youtube.com" || host === "m.youtube.com") && url.searchParams.get("v")) return "YouTube video";
  if (host === "youtu.be" && parts[0]) return "YouTube video";
  return undefined;
}

/* ---------- Colours ---------- */

export interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

export function parseColor(text: string): Rgba | null {
  const s = text.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.exec(s);
  if (hex) {
    let h = hex[1];
    if (h.length <= 4) h = [...h].map((c) => c + c).join("");
    const n = (i: number) => parseInt(h.slice(i, i + 2), 16);
    return { r: n(0), g: n(2), b: n(4), a: h.length === 8 ? +(n(6) / 255).toFixed(3) : 1 };
  }
  const rgb = /^rgba?\(\s*(\d{1,3})[\s,]+(\d{1,3})[\s,]+(\d{1,3})(?:[\s,/]+([\d.]+%?))?\s*\)$/.exec(s);
  if (rgb) {
    const [r, g, b] = [rgb[1], rgb[2], rgb[3]].map(Number);
    if ([r, g, b].some((v) => v > 255)) return null;
    return { r, g, b, a: alpha(rgb[4]) };
  }
  const hsl = /^hsla?\(\s*([\d.]+)(?:deg)?[\s,]+([\d.]+)%[\s,]+([\d.]+)%(?:[\s,/]+([\d.]+%?))?\s*\)$/.exec(s);
  if (hsl) {
    const { r, g, b } = hslToRgb(+hsl[1], +hsl[2], +hsl[3]);
    return { r, g, b, a: alpha(hsl[4]) };
  }
  return null;
}

function alpha(raw?: string): number {
  if (!raw) return 1;
  return raw.endsWith("%") ? +raw.slice(0, -1) / 100 : +raw;
}

function hslToRgb(h: number, s: number, l: number) {
  s /= 100;
  l /= 100;
  const k = (n: number) => (n + h / 30) % 12;
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)));
  return { r: Math.round(f(0) * 255), g: Math.round(f(8) * 255), b: Math.round(f(4) * 255) };
}

export function toHex({ r, g, b, a }: Rgba): string {
  const h = (v: number) => v.toString(16).padStart(2, "0");
  return `#${h(r)}${h(g)}${h(b)}${a < 1 ? h(Math.round(a * 255)) : ""}`.toUpperCase();
}

export function toRgbString({ r, g, b, a }: Rgba): string {
  return a < 1 ? `rgba(${r}, ${g}, ${b}, ${a})` : `rgb(${r}, ${g}, ${b})`;
}

export function toHslString({ r, g, b, a }: Rgba): string {
  const [rn, gn, bn] = [r / 255, g / 255, b / 255];
  const max = Math.max(rn, gn, bn);
  const min = Math.min(rn, gn, bn);
  const l = (max + min) / 2;
  let h = 0;
  let s = 0;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    h = max === rn ? (gn - bn) / d + (gn < bn ? 6 : 0) : max === gn ? (bn - rn) / d + 2 : (rn - gn) / d + 4;
    h *= 60;
  }
  const body = `${Math.round(h)}, ${Math.round(s * 100)}%, ${Math.round(l * 100)}%`;
  return a < 1 ? `hsla(${body}, ${a})` : `hsl(${body})`;
}

/** WCAG contrast ratio against white and black, for choosing text colour. */
export function contrast({ r, g, b }: Rgba): { white: number; black: number } {
  const lin = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const L = 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
  return { white: +(1.05 / (L + 0.05)).toFixed(2), black: +((L + 0.05) / 0.05).toFixed(2) };
}

/* ---------- Tokens and secrets ---------- */

function decodeBase64Url(part: string): string {
  const b64 = part.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(part.length / 4) * 4, "=");
  const bin = atob(b64);
  return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}

export interface JwtInfo {
  header: Record<string, unknown>;
  payload: Record<string, unknown>;
  expires?: Date;
  issued?: Date;
}

export function parseJwt(text: string): JwtInfo | null {
  if (!/^[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*$/.test(text)) return null;
  try {
    const [h, p] = text.split(".");
    const header = JSON.parse(decodeBase64Url(h));
    const payload = JSON.parse(decodeBase64Url(p));
    if (typeof header !== "object" || !header || !("alg" in header)) return null;
    const date = (v: unknown) => (typeof v === "number" ? new Date(v * 1000) : undefined);
    return { header, payload, expires: date(payload.exp), issued: date(payload.iat) };
  } catch {
    return null;
  }
}

export interface SecretKind {
  /** Title for chips and lists: "AWS access key". */
  label: string;
  /** The same thing mid-sentence, with its article: "an AWS access key". */
  noun: string;
}

const SECRETS: [string, string, RegExp][] = [
  ["Private key", "a private key", /-----BEGIN (?:[A-Z]+ )*PRIVATE KEY-----/],
  ["AWS access key", "an AWS access key", /\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/],
  ["GitHub token", "a GitHub token", /\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,})\b/],
  ["Anthropic API key", "an Anthropic API key", /\bsk-ant-[A-Za-z0-9_-]{20,}/],
  ["OpenAI API key", "an OpenAI API key", /\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}/],
  ["Stripe secret key", "a Stripe secret key", /\b[rs]k_live_[A-Za-z0-9]{16,}/],
  ["Slack token", "a Slack token", /\bxox[abprs]-[A-Za-z0-9-]{10,}/],
  ["Google API key", "a Google API key", /\bAIza[0-9A-Za-z_-]{35}\b/],
  ["Connection string", "a connection string with a password", /\b[a-z][a-z0-9+.-]*:\/\/[^\s:/@]+:[^\s@/]+@[^\s]+/i],
];

function entropy(s: string): number {
  const counts = new Map<string, number>();
  for (const ch of s) counts.set(ch, (counts.get(ch) ?? 0) + 1);
  let e = 0;
  for (const n of counts.values()) e -= (n / s.length) * Math.log2(n / s.length);
  return e;
}

/** The kind of secret this looks like, or null. */
export function detectSecret(text: string): SecretKind | null {
  for (const [label, noun, re] of SECRETS) if (re.test(text)) return { label, noun };
  // A lone, dense, mixed-character token is probably a password.
  const t = text.trim();
  if (/^\S{10,64}$/.test(t) && !/^[a-z][a-z0-9+.-]*:\/\//i.test(t)) {
    const mixed = /[a-z]/.test(t) && /[A-Z]/.test(t) && /\d/.test(t);
    const symbol = /[!@#$%^&*+=?~]/.test(t);
    if ((mixed || (symbol && /\d/.test(t))) && entropy(t) >= 3.2) return { label: "Password", noun: "a password" };
  }
  return null;
}

/* ---------- Time ---------- */

export interface TimeInfo {
  date: Date;
  source: "unix seconds" | "unix milliseconds" | "ISO 8601";
}

const MIN_S = 946_684_800; // 2000-01-01
const MAX_S = 4_102_444_800; // 2100-01-01

export function parseTime(text: string): TimeInfo | null {
  const t = text.trim();
  if (/^\d{10}$/.test(t) && +t >= MIN_S && +t <= MAX_S) return { date: new Date(+t * 1000), source: "unix seconds" };
  if (/^\d{13}$/.test(t) && +t >= MIN_S * 1000 && +t <= MAX_S * 1000) return { date: new Date(+t), source: "unix milliseconds" };
  if (/^\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?)?$/.test(t)) {
    const d = new Date(t);
    if (!Number.isNaN(d.getTime())) return { date: d, source: "ISO 8601" };
  }
  return null;
}

/* ---------- Tables ---------- */

/** RFC 4180-style split of one line: handles quoted fields and doubled quotes. */
function splitRow(line: string, delim: string): string[] {
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (quoted) {
      if (ch === '"' && line[i + 1] === '"') {
        cur += '"';
        i++;
      } else if (ch === '"') quoted = false;
      else cur += ch;
    } else if (ch === '"' && cur === "") quoted = true;
    else if (ch === delim) {
      out.push(cur.trim());
      cur = "";
    } else cur += ch;
  }
  out.push(cur.trim());
  return out;
}

export interface TableInfo {
  rows: string[][];
  delimiter: "tab" | "comma" | "semicolon";
}

export function parseTable(text: string): TableInfo | null {
  const lines = text.trim().split(/\r?\n/).filter((l) => l.trim());
  const options: [string, TableInfo["delimiter"], number][] = [
    ["\t", "tab", 2],
    [",", "comma", 3],
    [";", "semicolon", 3],
  ];
  for (const [d, name, minLines] of options) {
    if (lines.length < minLines) continue;
    const rows = lines.map((l) => splitRow(l, d));
    const cols = rows[0].length;
    if (cols >= 2 && rows.every((r) => r.length === cols) && !lines.some((l) => /[{};]\s*$/.test(l) && d !== ";")) return { rows, delimiter: name };
  }
  return null;
}

/* ---------- File paths ---------- */

export interface PathInfo {
  style: "windows" | "unc" | "posix";
  segments: string[];
  name: string;
  extension?: string;
  /** The same path written for the other world (WSL / Windows). */
  alternate?: { label: string; value: string };
}

export function parsePath(text: string): PathInfo | null {
  const t = text.trim();
  if (t.includes("\n") || t.length > 1024) return null;
  let style: PathInfo["style"];
  let segments: string[];
  if (/^[a-zA-Z]:\\[^<>:"|?*\n]*$/.test(t)) {
    style = "windows";
    segments = t.split("\\").filter(Boolean);
  } else if (/^\\\\[^\\\s]+\\[^\n]*$/.test(t)) {
    style = "unc";
    segments = t.slice(2).split("\\").filter(Boolean);
  } else if (/^(~|\/)[^\s<>|]*\/[^\s<>|]*$/.test(t) && !t.startsWith("//")) {
    style = "posix";
    segments = t.split("/").filter(Boolean);
    if (t.startsWith("~")) segments[0] = "~";
  } else return null;
  const name = segments[segments.length - 1] ?? t;
  const ext = /\.([A-Za-z0-9]{1,8})$/.exec(name)?.[1];
  let alternate: PathInfo["alternate"];
  if (style === "windows") alternate = { label: "WSL path", value: `/mnt/${t[0].toLowerCase()}/${segments.slice(1).join("/")}` };
  const wsl = /^\/mnt\/([a-z])\/(.*)$/.exec(t);
  if (wsl) alternate = { label: "Windows path", value: `${wsl[1].toUpperCase()}:\\${wsl[2].split("/").join("\\")}` };
  return { style, segments, name, extension: ext?.toLowerCase(), alternate };
}

/* ---------- Arithmetic ---------- */

/** Evaluates + - * / % ^ and parentheses with a tiny recursive-descent parser. No eval. */
export function evaluate(expr: string): number | null {
  const src = expr.replace(/×/g, "*").replace(/÷/g, "/").replace(/,(?=\d{3}\b)/g, "");
  let i = 0;
  const peek = () => src[i];
  const skip = () => {
    while (src[i] === " ") i++;
  };
  const number = (): number => {
    skip();
    const m = /^\d+(\.\d+)?|^\.\d+/.exec(src.slice(i));
    if (!m) throw new Error("number expected");
    i += m[0].length;
    return parseFloat(m[0]);
  };
  const atom = (): number => {
    skip();
    if (peek() === "-") {
      i++;
      return -atom();
    }
    if (peek() === "(") {
      i++;
      const v = sum();
      skip();
      if (peek() !== ")") throw new Error(") expected");
      i++;
      return v;
    }
    return number();
  };
  const power = (): number => {
    const base = atom();
    skip();
    if (peek() === "^") {
      i++;
      return base ** power();
    }
    return base;
  };
  const product = (): number => {
    let v = power();
    for (;;) {
      skip();
      const op = peek();
      if (op !== "*" && op !== "/" && op !== "%") return v;
      i++;
      const r = power();
      v = op === "*" ? v * r : op === "/" ? v / r : v % r;
    }
  };
  const sum = (): number => {
    let v = product();
    for (;;) {
      skip();
      const op = peek();
      if (op !== "+" && op !== "-") return v;
      i++;
      const r = product();
      v = op === "+" ? v + r : v - r;
    }
  };
  try {
    const v = sum();
    skip();
    return i === src.length && Number.isFinite(v) ? v : null;
  } catch {
    return null;
  }
}

/** Arithmetic that a person would want answered: has an operator, isn't a phone number or date. */
export function parseMath(text: string): { expression: string; result: number } | null {
  const t = text.trim();
  if (t.length > 200 || !/^[\d\s.,+\-*/%^()×÷]+$/.test(t)) return null;
  if (!/\d\s*([+*/%^×÷]|\s-\s)\s*[\d(]/.test(t)) return null;
  const result = evaluate(t);
  return result === null ? null : { expression: t, result };
}

/* ---------- Plain text ---------- */

export interface TextStats {
  words: number;
  chars: number;
  lines: number;
  /** Lines broken mid-sentence, as when copying from a PDF. */
  hardWrapped: boolean;
}

export function textStats(text: string): TextStats {
  const lines = text.split(/\r?\n/);
  const words = text.trim() ? text.trim().split(/\s+/).length : 0;
  const breaks = lines.slice(0, -1).filter((l, i) => l.trim() && !/[.!?:;"”)]\s*$/.test(l) && /^\s*[a-z]/.test(lines[i + 1] ?? "")).length;
  return { words, chars: text.length, lines: lines.length, hardWrapped: lines.length >= 3 && breaks >= Math.max(2, (lines.length - 1) * 0.5) };
}

/** Joins lines broken mid-sentence, keeping paragraph breaks. */
export function joinLines(text: string): string {
  return text
    .split(/\n\s*\n/)
    .map((p) => p.replace(/-\n(?=[a-z])/g, "").replace(/\s*\n\s*/g, " ").trim())
    .join("\n\n");
}

export function titleCase(text: string): string {
  const small = /^(a|an|and|as|at|but|by|for|in|of|on|or|the|to|vs?\.?|via)$/i;
  return text.toLowerCase().replace(/\b([a-z])([\w']*)/g, (w, first: string, rest: string, offset: number) => (offset > 0 && small.test(w) ? w : first.toUpperCase() + rest));
}
