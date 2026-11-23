// One preview per kind of clipboard content. Each shows the thing the way that
// kind of thing should be read, and offers the conversions people reach for.

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useTether } from "@/data/store";
import type { Content } from "@/lib/content/detect";
import { formatNumber } from "@/lib/content/detect";
import { formatCode, type Formatted } from "@/lib/content/format";
import { highlight, type Language } from "@/lib/content/language";
import { contrast, joinLines, titleCase, toHex, toHslString, toRgbString, type JwtInfo, type LinkInfo, type PathInfo, type Rgba, type TableInfo, type TextStats, type TimeInfo } from "@/lib/content/parsers";
import { relativeTime } from "@/lib/format";
import { os } from "@/lib/platform";
import { cn } from "@/lib/cn";
import { Icon, icons } from "@/components/Icon";
import { Button, Chip, IconButton, Panel, Segmented } from "@/components/ui";

export function useCopy() {
  const toast = useTether((s) => s.toast);
  return useCallback(
    (text: string, message = "Copied") =>
      void navigator.clipboard.writeText(text).then(
        () => toast(message),
        () => toast("Clipboard access was blocked"),
      ),
    [toast],
  );
}

function Label({ children }: { children: ReactNode }) {
  return <p className="mb-2 text-xs font-medium text-muted-foreground">{children}</p>;
}

/** A labelled value with a copy button, for conversions. */
function CopyRow({ label, value, mono = true }: { label: string; value: string; mono?: boolean }) {
  const copy = useCopy();
  return (
    <div className="flex items-center gap-4 px-4 py-2.5">
      <span className="w-28 shrink-0 text-xs text-muted-foreground">{label}</span>
      <span className={cn("selectable min-w-0 flex-1 truncate text-[13px]", mono && "font-mono text-[12.5px]")}>{value}</span>
      <IconButton icon={icons.copy} label={`Copy ${label}`} onClick={() => copy(value, `Copied ${label}`)} />
    </div>
  );
}

/* ---------- Code and JSON ---------- */

export function CodeBlock({ code, language, maxHeight = "46vh" }: { code: string; language: Language; maxHeight?: string }) {
  const html = useMemo(() => highlight(code, language), [code, language]);
  const lines = useMemo(() => code.split("\n").length, [code]);
  return (
    <div className="code overflow-auto rounded-2xl border border-border bg-surface-2" style={{ maxHeight }}>
      <div className="flex min-w-max font-mono text-[12.5px] leading-[1.75]">
        <div aria-hidden className="sticky left-0 border-r border-border bg-surface-2 py-4 pr-3 pl-4 text-right text-subtle select-none">
          {Array.from({ length: lines }, (_, i) => (
            <div key={i}>{i + 1}</div>
          ))}
        </div>
        {/* highlight.js escapes the source, so the markup is safe. */}
        <pre className="selectable py-4 pr-6 pl-4">
          <code dangerouslySetInnerHTML={{ __html: html }} />
        </pre>
      </div>
    </div>
  );
}

export function CodeView({ text, language }: { text: string; language: Language }) {
  const copy = useCopy();
  // undefined while formatting runs; null when there is nothing to improve.
  const [formatted, setFormatted] = useState<Formatted | null | undefined>(undefined);
  const [view, setView] = useState<"formatted" | "original">("formatted");

  useEffect(() => {
    let live = true;
    setFormatted(undefined);
    setView("formatted");
    void formatCode(text, language).then((f) => live && setFormatted(f));
    return () => {
      live = false;
    };
  }, [text, language]);

  const showing = view === "formatted" && formatted ? formatted.text : text;
  const status = formatted === undefined ? "Formatting…" : formatted ? (view === "formatted" ? formatted.tool : "As copied") : "Already well formatted";

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
          {formatted && view === "formatted" && <Icon icon={icons.wand} size={13} />}
          {status}
        </span>
        {formatted && (
          <div className="flex items-center gap-2">
            <Segmented
              label="Code view"
              value={view}
              onChange={setView}
              options={[
                { value: "formatted", label: "Formatted" },
                { value: "original", label: "Original" },
              ]}
            />
            <Button size="sm" variant="outline" icon={icons.copy} onClick={() => copy(formatted.text, "Copied formatted code")}>
              Copy formatted
            </Button>
          </div>
        )}
      </div>
      <CodeBlock code={showing} language={language} />
    </div>
  );
}

/* ---------- Links ---------- */

export function LinkView({ link }: { link: LinkInfo }) {
  const copy = useCopy();
  const openLink = useTether((s) => s.backend.openLink);
  const { url } = link;
  const host = url.hostname.replace(/^www\./, "");
  const params = [...url.searchParams.entries()];
  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-start gap-4">
        <span className="flex size-12 shrink-0 items-center justify-center rounded-2xl bg-muted text-lg font-semibold text-muted-foreground uppercase">{host[0]}</span>
        <div className="min-w-0">
          <p className="truncate text-[17px] font-semibold tracking-tight">{link.title ?? host}</p>
          <p className="selectable mt-0.5 text-[13px] break-all text-muted-foreground">
            <span className="text-foreground">{host}</span>
            {url.pathname !== "/" && url.pathname}
          </p>
        </div>
      </div>
      {params.length > 0 && (
        <div>
          <Label>Query parameters</Label>
          <Panel className="divide-y divide-border">
            {params.map(([k, v]) => {
              const tracker = link.tracking.includes(k);
              return (
                <div key={k} className="flex items-center gap-4 px-4 py-2.5 font-mono text-[12.5px]">
                  <span className={cn("w-40 shrink-0 truncate", tracker ? "text-subtle" : "text-foreground")}>{k}</span>
                  <span className={cn("selectable min-w-0 flex-1 truncate", tracker ? "text-subtle line-through" : "text-muted-foreground")}>{v || "—"}</span>
                  {tracker && <Chip>Tracker</Chip>}
                </div>
              );
            })}
          </Panel>
        </div>
      )}
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" icon={icons.external} onClick={() => openLink(url.href)}>
          Open in browser
        </Button>
        {link.tracking.length > 0 && (
          <Button variant="outline" size="sm" icon={icons.wand} onClick={() => copy(link.clean, "Copied without trackers")}>
            Copy without trackers
          </Button>
        )}
      </div>
    </div>
  );
}

/* ---------- Colour ---------- */

const grade = (r: number) => (r >= 7 ? "AAA" : r >= 4.5 ? "AA" : r >= 3 ? "AA large" : "Fails");

export function ColorView({ color }: { color: Rgba }) {
  const c = contrast(color);
  const ink = c.white >= c.black ? "#ffffff" : "#000000";
  return (
    <div className="grid gap-5 md:grid-cols-[220px_minmax(0,1fr)]">
      <div className="flex h-48 flex-col justify-between rounded-2xl border border-border p-4" style={{ background: toRgbString(color), color: ink }}>
        <span className="text-xs font-medium opacity-80">{toHex(color)}</span>
        <span className="text-3xl font-semibold tracking-tight">Aa</span>
      </div>
      <div className="flex flex-col gap-4">
        <Panel className="divide-y divide-border">
          <CopyRow label="HEX" value={toHex(color)} />
          <CopyRow label="RGB" value={toRgbString(color)} />
          <CopyRow label="HSL" value={toHslString(color)} />
        </Panel>
        <div className="flex flex-wrap gap-2 text-xs">
          <Chip>
            On white {c.white}:1 · {grade(c.white)}
          </Chip>
          <Chip>
            On black {c.black}:1 · {grade(c.black)}
          </Chip>
        </div>
      </div>
    </div>
  );
}

/* ---------- Tokens and secrets ---------- */

export function JwtView({ jwt }: { jwt: JwtInfo }) {
  const expired = jwt.expires ? jwt.expires.getTime() < Date.now() : false;
  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-center gap-2">
        {jwt.expires && <Chip className={expired ? "text-destructive" : "text-positive"}>{expired ? `Expired ${relativeTime(jwt.expires)}` : `Valid, expires ${relativeTime(jwt.expires)}`}</Chip>}
        {jwt.issued && <Chip>Issued {relativeTime(jwt.issued)}</Chip>}
        <Chip>{String(jwt.header.alg)}</Chip>
      </div>
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.6fr)]">
        <div>
          <Label>Header</Label>
          <CodeBlock code={JSON.stringify(jwt.header, null, 2)} language="json" maxHeight="30vh" />
        </div>
        <div>
          <Label>Payload</Label>
          <CodeBlock code={JSON.stringify(jwt.payload, null, 2)} language="json" maxHeight="30vh" />
        </div>
      </div>
      <p className="text-xs text-subtle">Decoded locally. The signature is not verified.</p>
    </div>
  );
}

export function SecretView({ text }: { text: string }) {
  return <pre className="selectable overflow-auto rounded-2xl border border-border bg-surface-2 p-4 font-mono text-[13px] break-all whitespace-pre-wrap">{text}</pre>;
}

/** Shown instead of the preview until the user chooses to look. */
export function SecretGate({ noun, text, onReveal }: { noun: string; text: string; onReveal: () => void }) {
  const t = text.trim();
  const masked = t.length > 12 ? `${t.slice(0, 4)}${"•".repeat(Math.min(16, t.length - 8))}${t.slice(-4)}` : "•".repeat(10);
  return (
    <div className="flex flex-col items-start gap-4 rounded-2xl border border-border bg-surface-2 p-5">
      <div className="flex items-center gap-3">
        <span className="flex size-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <Icon icon={icons.lock} size={18} />
        </span>
        <div>
          <p className="text-sm font-medium">This looks like {noun}.</p>
          <p className="text-[13px] text-muted-foreground">Tether keeps it on this device and never syncs it.</p>
        </div>
      </div>
      <code className="font-mono text-[13px] tracking-wider text-muted-foreground">{masked}</code>
      <Button variant="outline" size="sm" icon={icons.hide} onClick={onReveal}>
        Reveal
      </Button>
    </div>
  );
}

/* ---------- Time ---------- */

export function TimeView({ time }: { time: TimeInfo }) {
  const d = time.date;
  return (
    <div className="flex flex-col gap-5">
      <div>
        <p className="display text-[32px]">{d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}</p>
        <p className="mt-1 text-[15px] text-muted-foreground">
          {d.toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "long", day: "numeric" })} · {relativeTime(d)}
        </p>
      </div>
      <Panel className="divide-y divide-border">
        <CopyRow label="Local" value={d.toLocaleString()} mono={false} />
        <CopyRow label="UTC" value={d.toUTCString()} mono={false} />
        <CopyRow label="ISO 8601" value={d.toISOString()} />
        <CopyRow label="Unix seconds" value={String(Math.floor(d.getTime() / 1000))} />
        <CopyRow label="Unix ms" value={String(d.getTime())} />
      </Panel>
      <p className="text-xs text-subtle">Read as {time.source}.</p>
    </div>
  );
}

/* ---------- Table ---------- */

export function TableView({ table }: { table: TableInfo }) {
  const copy = useCopy();
  const [head, ...rows] = table.rows;
  const markdown = () => [head, head.map(() => "---"), ...rows].map((r) => `| ${r.map((c) => c.replace(/\|/g, "\\|")).join(" | ")} |`).join("\n");
  const json = () => JSON.stringify(rows.map((r) => Object.fromEntries(head.map((h, i) => [h, r[i]]))), null, 2);
  return (
    <div className="flex flex-col gap-4">
      <div className="max-h-[46vh] overflow-auto rounded-2xl border border-border">
        <table className="selectable w-full text-[13px]">
          <thead className="sticky top-0 bg-surface-2">
            <tr>
              {head.map((h, i) => (
                <th key={i} className="border-b border-border px-4 py-2.5 text-left text-xs font-semibold whitespace-nowrap">
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, 200).map((r, i) => (
              <tr key={i} className="border-b border-border last:border-0">
                {r.map((c, j) => (
                  <td key={j} className={cn("px-4 py-2", /^-?[\d.,]+%?$/.test(c) && "text-right font-mono text-[12.5px]")}>
                    {c}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" icon={icons.copy} onClick={() => copy(markdown(), "Copied as a Markdown table")}>
          Copy as Markdown
        </Button>
        <Button size="sm" variant="outline" icon={icons.braces} onClick={() => copy(json(), "Copied as JSON")}>
          Copy as JSON
        </Button>
      </div>
    </div>
  );
}

/* ---------- Paths ---------- */

// No "Show in File Explorer" yet: a path can come from another device, and
// asking Windows to show a network path (\\host\share) makes it sign in to
// that host, which leaks the user's NTLM hash. It needs a local-paths-only
// check first.
export function PathView({ path }: { path: PathInfo }) {
  const copy = useCopy();
  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-center gap-3">
        <span className="flex size-12 shrink-0 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
          <Icon icon={path.extension ? icons.file : icons.folder} size={20} />
        </span>
        <div className="min-w-0">
          <p className="truncate text-[17px] font-semibold tracking-tight">{path.name}</p>
          <p className="text-[13px] text-muted-foreground">{path.style === "windows" ? "Windows path" : path.style === "unc" ? "Network share" : "Unix path"}</p>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-1 text-[13px]">
        {path.segments.map((s, i) => (
          <span key={i} className="inline-flex items-center gap-1">
            {i > 0 && <Icon icon={icons.chevronRight} size={12} className="text-subtle" />}
            <span className={cn("rounded-lg px-2 py-0.5", i === path.segments.length - 1 ? "bg-muted text-foreground" : "text-muted-foreground")}>{s}</span>
          </span>
        ))}
      </div>
      {path.alternate && (
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant="outline" icon={icons.copy} onClick={() => copy(path.alternate!.value, `Copied ${path.alternate!.label}`)}>
            Copy as {path.alternate.label}
          </Button>
        </div>
      )}
    </div>
  );
}

/* ---------- Small kinds ---------- */

export function EmailView({ address }: { address: string }) {
  const openLink = useTether((s) => s.backend.openLink);
  return (
    <div className="flex flex-col items-start gap-4">
      <p className="selectable text-[22px] font-semibold tracking-tight">{address}</p>
      <Button variant="outline" size="sm" icon={icons.mail} onClick={() => openLink(`mailto:${address}`)}>
        Write an email
      </Button>
    </div>
  );
}

export function MathView({ expression, result }: { expression: string; result: number }) {
  const copy = useCopy();
  return (
    <div className="flex flex-col items-start gap-3">
      <p className="selectable font-mono text-[15px] text-muted-foreground">{expression}</p>
      <p className="display text-[44px]">= {formatNumber(result)}</p>
      <Button size="sm" variant="outline" icon={icons.copy} onClick={() => copy(String(result), "Copied the result")}>
        Copy result
      </Button>
    </div>
  );
}

export function UuidView({ text, version }: { text: string; version: number }) {
  const id = text.trim().toLowerCase();
  return (
    <div className="flex flex-col gap-4">
      <p className="text-[13px] text-muted-foreground">UUID version {version}</p>
      <Panel className="divide-y divide-border">
        <CopyRow label="Lowercase" value={id} />
        <CopyRow label="Uppercase" value={id.toUpperCase()} />
        <CopyRow label="No hyphens" value={id.replace(/-/g, "")} />
        <CopyRow label={os === "windows" ? "Registry GUID" : "Braced"} value={`{${id.toUpperCase()}}`} />
      </Panel>
    </div>
  );
}

/* ---------- Text ---------- */

const sentenceCase = (t: string) => t.toLowerCase().replace(/(^\s*\w|[.!?]\s+\w)/g, (m) => m.toUpperCase());

export function TextView({ text, stats }: { text: string; stats: TextStats }) {
  const copy = useCopy();
  const [joined, setJoined] = useState(false);
  const shown = joined ? joinLines(text) : text;
  const transforms: [string, () => string][] = [
    ["UPPERCASE", () => shown.toUpperCase()],
    ["lowercase", () => shown.toLowerCase()],
    ["Title Case", () => titleCase(shown)],
    ["Sentence case", () => sentenceCase(shown)],
  ];
  return (
    <div className="flex flex-col gap-5">
      {stats.hardWrapped && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-border bg-surface-2 px-4 py-3">
          <span className="inline-flex items-center gap-2 text-[13px] text-muted-foreground">
            <Icon icon={icons.wrap} size={15} />
            Lines break mid-sentence, as text copied from a PDF does.
          </span>
          <div className="flex gap-2">
            <Button size="sm" variant="outline" onClick={() => setJoined((j) => !j)}>
              {joined ? "Show original" : "Join lines"}
            </Button>
            {joined && (
              <Button size="sm" variant="solid" icon={icons.copy} onClick={() => copy(shown, "Copied with lines joined")}>
                Copy joined
              </Button>
            )}
          </div>
        </div>
      )}
      <p className="selectable text-[17px] leading-relaxed whitespace-pre-wrap">{shown}</p>
      <div className="flex flex-wrap items-center gap-2 border-t border-border pt-4">
        <span className="mr-1 text-xs text-subtle">Copy as</span>
        {transforms.map(([label, run]) => (
          <Button key={label} size="sm" variant="ghost" onClick={() => copy(run(), `Copied as ${label}`)}>
            {label}
          </Button>
        ))}
        <span className="ml-auto text-xs text-subtle">
          {stats.words.toLocaleString()} words · {stats.chars.toLocaleString()} characters
        </span>
      </div>
    </div>
  );
}

/** Picks the preview for a detected kind. Sensitive kinds are gated by the caller. */
export function Preview({ content, text }: { content: Content; text: string }) {
  switch (content.kind) {
    case "json":
      return <CodeView text={text} language="json" />;
    case "code":
      return <CodeView text={text} language={content.language} />;
    case "link":
      return <LinkView link={content.link} />;
    case "color":
      return <ColorView color={content.color} />;
    case "jwt":
      return <JwtView jwt={content.jwt} />;
    case "secret":
      return <SecretView text={text} />;
    case "time":
      return <TimeView time={content.time} />;
    case "table":
      return <TableView table={content.table} />;
    case "path":
      return <PathView path={content.path} />;
    case "email":
      return <EmailView address={content.address} />;
    case "math":
      return <MathView expression={content.expression} result={content.result} />;
    case "uuid":
      return <UuidView text={text} version={content.version} />;
    case "text":
      return <TextView text={text} stats={content.stats} />;
  }
}
