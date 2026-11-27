// Renders a note's Markdown with React elements only: nothing from the note
// becomes HTML (code highlighting comes from highlight.js, which escapes its
// input). Links open only through the app's link check.

import { Fragment, useMemo } from "react";
import { useTether } from "@/data/store";
import { detectLanguage, type Language } from "@/lib/content/language";
import { parse, toggled, type Inline, type ListItem } from "@/lib/notes/markdown";
import { cn } from "@/lib/cn";
import { CodeBlock } from "../clipboard/previews";

const ALIASES: Record<string, Language> = {
  js: "javascript",
  javascript: "javascript",
  jsx: "javascript",
  ts: "typescript",
  tsx: "typescript",
  typescript: "typescript",
  sh: "bash",
  shell: "bash",
  bash: "bash",
  zsh: "bash",
  py: "python",
  python: "python",
  rs: "rust",
  rust: "rust",
  go: "go",
  java: "java",
  c: "cpp",
  cpp: "cpp",
  "c++": "cpp",
  cs: "csharp",
  csharp: "csharp",
  css: "css",
  html: "xml",
  xml: "xml",
  json: "json",
  yml: "yaml",
  yaml: "yaml",
  sql: "sql",
  ps1: "powershell",
  pwsh: "powershell",
  powershell: "powershell",
  ini: "ini",
  toml: "ini",
  dockerfile: "dockerfile",
  md: "markdown",
  markdown: "markdown",
};

function Inlines({ parts }: { parts: Inline[] }) {
  const openLink = useTether((s) => s.backend.openLink);
  return (
    <>
      {parts.map((p, i) => {
        switch (p.t) {
          case "text":
            return <Fragment key={i}>{p.v}</Fragment>;
          case "code":
            return (
              <code key={i} className="inline">
                {p.v}
              </code>
            );
          case "strong":
            return (
              <strong key={i}>
                <Inlines parts={p.c} />
              </strong>
            );
          case "em":
            return (
              <em key={i}>
                <Inlines parts={p.c} />
              </em>
            );
          case "del":
            return (
              <del key={i}>
                <Inlines parts={p.c} />
              </del>
            );
          case "link":
            // A button, not an <a href>: the window never navigates, and the
            // app checks the link before the browser gets it.
            return (
              <button key={i} type="button" className="link" title={p.href} onClick={() => openLink(p.href)}>
                <Inlines parts={p.c} />
              </button>
            );
        }
      })}
    </>
  );
}

function Item({ item, onToggle }: { item: ListItem; onToggle?: (item: ListItem) => void }) {
  return (
    <li className="flex items-start gap-2.5" style={{ paddingLeft: item.depth * 22 }}>
      {item.checked !== null ? (
        <input
          type="checkbox"
          checked={item.checked}
          disabled={!onToggle}
          onChange={() => onToggle?.(item)}
          className="mt-[0.42em] size-4 shrink-0 accent-[var(--positive)]"
          aria-label={item.checked ? "Done" : "Not done"}
        />
      ) : (
        <span className="mt-[0.05em] min-w-4 shrink-0 text-right text-muted-foreground tabular-nums">{item.number !== null ? `${item.number}.` : "•"}</span>
      )}
      <span className={cn("min-w-0", item.checked && "done")}>
        <Inlines parts={item.inline} />
      </span>
    </li>
  );
}

/** `onToggle(box, replacement)`: tick or untick a checklist item in the note. */
export function MarkdownView({ text, onToggle }: { text: string; onToggle?: (box: number, replacement: string) => void }) {
  const blocks = useMemo(() => parse(text), [text]);
  const toggle = onToggle ? (item: ListItem) => onToggle(item.box, toggled(item.checked === true)) : undefined;
  if (!text.trim()) return <p className="text-muted-foreground">Nothing written yet.</p>;
  return (
    <div className="note-read selectable">
      {blocks.map((b, i) => {
        switch (b.kind) {
          case "heading": {
            const H = `h${Math.min(4, b.level)}` as "h1" | "h2" | "h3" | "h4";
            return (
              <H key={i}>
                <Inlines parts={b.inline} />
              </H>
            );
          }
          case "para":
            return (
              <p key={i} className="whitespace-pre-wrap">
                <Inlines parts={b.inline} />
              </p>
            );
          case "quote":
            return (
              <blockquote key={i} className="whitespace-pre-wrap">
                <Inlines parts={b.inline} />
              </blockquote>
            );
          case "rule":
            return <hr key={i} />;
          case "list":
            return (
              <ul key={i} className="flex flex-col gap-1">
                {b.items.map((item, k) => (
                  <Item key={k} item={item} onToggle={toggle} />
                ))}
              </ul>
            );
          case "code": {
            const language = ALIASES[b.lang] ?? (b.lang ? null : detectLanguage(b.text));
            return language ? (
              <div key={i} className="code">
                <CodeBlock code={b.text} language={language} maxHeight="60vh" />
              </div>
            ) : (
              <pre key={i} className="code overflow-auto rounded-2xl border border-border bg-surface-2 px-4 py-3 font-mono text-[12.5px]">
                {b.text}
              </pre>
            );
          }
        }
      })}
    </div>
  );
}
