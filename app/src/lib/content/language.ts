// Programming-language detection and highlighting for clipboard code.
// Cheap, high-precision heuristics run first; highlight.js auto-detection over a
// fixed subset of languages is the fallback for anything they miss.

import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import powershell from "highlight.js/lib/languages/powershell";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

const LANGS = { bash, cpp, csharp, css, dockerfile, go, ini, java, javascript, json, markdown, powershell, python, rust, sql, typescript, xml, yaml } as const;
for (const [name, def] of Object.entries(LANGS)) hljs.registerLanguage(name, def);

export type Language = keyof typeof LANGS;

export const languageName: Record<Language, string> = {
  bash: "Shell",
  cpp: "C++",
  csharp: "C#",
  css: "CSS",
  dockerfile: "Dockerfile",
  go: "Go",
  ini: "TOML / INI",
  java: "Java",
  javascript: "JavaScript",
  json: "JSON",
  markdown: "Markdown",
  powershell: "PowerShell",
  python: "Python",
  rust: "Rust",
  sql: "SQL",
  typescript: "TypeScript",
  xml: "HTML / XML",
  yaml: "YAML",
};

/** Ordered: the first rule that matches wins, so more specific languages come first. */
const RULES: [Language, RegExp[]][] = [
  ["dockerfile", [/^FROM\s+\S+/m, /^(RUN|COPY|WORKDIR|ENTRYPOINT|CMD)\s/m]],
  ["sql", [/^\s*(SELECT|INSERT\s+INTO|UPDATE\s+\w+\s+SET|DELETE\s+FROM|CREATE\s+(TABLE|INDEX|VIEW)|ALTER\s+TABLE|WITH\s+\w+\s+AS)\b/i]],
  ["rust", [/\bfn\s+\w+\s*(<[^>]*>)?\s*\(/, /\blet\s+mut\b/, /\bimpl(\s*<[^>]*>)?\s+\w+/, /#\[(derive|cfg|test)/, /\b(pub\s+(fn|struct|enum|mod)|use\s+\w+::)/]],
  ["go", [/^package\s+\w+/m, /\bfunc\s+(\(\w+\s+\*?\w+\)\s*)?\w+\(/, /\w+\s*:=\s*/]],
  ["python", [/^\s*def\s+\w+\(.*\)\s*(->\s*[\w\[\], .]+)?:\s*$/m, /^\s*class\s+\w+(\(.*\))?:\s*$/m, /^\s*(from\s+[\w.]+\s+import|import\s+\w+(\s+as\s+\w+)?$)/m, /^\s*(elif|except\s*\w*|with\s+.*\s+as\s+\w+):/m]],
  ["csharp", [/\busing\s+System(\.\w+)*;/, /\bnamespace\s+[\w.]+/, /\b(public|private|internal)\s+(static\s+)?(async\s+)?(class|void|Task|string|int)\b.*[({]/]],
  ["java", [/\bpublic\s+static\s+void\s+main\s*\(/, /\bSystem\.out\.print/, /^import\s+java\./m]],
  ["cpp", [/^#include\s*[<"]/m, /\bstd::\w+/]],
  ["powershell", [/\b(Get|Set|New|Remove|Start|Stop|Invoke|Write)-[A-Z]\w+/, /^\s*\$\w+\s*=\s*/m]],
  ["typescript", [/\b(interface|type)\s+\w+\s*(<[^>]*>)?\s*[={]/, /\b(const|let|function)\b[^\n]*:\s*(string|number|boolean|void|unknown|any)\b/, /\bas\s+const\b/, /import\s+type\s/]],
  ["javascript", [/\b(const|let|var)\s+\w+\s*=/, /=>\s*[{(]?/, /\bfunction\s*\w*\s*\(/, /^\s*(import|export)\s.+from\s+['"]/m, /\bconsole\.\w+\(/]],
  ["xml", [/^\s*<(!DOCTYPE|html|\?xml|div|svg|[a-zA-Z][\w-]*(\s+[\w-]+=("[^"]*"|'[^']*'))*\s*\/?>)/i]],
  ["css", [/^\s*[@.#]?[\w-][\w\s.#:>,-]*\{\s*[\w-]+\s*:[^;{}]+;/m]],
  ["yaml", [/^[\w-]+:\s*(\S.*)?$(\n^(\s{2,}|-\s)[^\n]*$)+/m]],
  ["ini", [/^\[[\w.-]+\]\s*$/m]],
  ["markdown", [/^#{1,6}\s+\S/m, /^```/m, /^\s*[-*]\s+\[[ x]\]\s/m]],
  ["bash", [/^\s*(\$\s+)?(sudo|cd|ls|git|npm|npx|pnpm|yarn|cargo|docker|kubectl|curl|wget|echo|export|chmod|mkdir|rm|cp|mv|ssh|brew|apt|pip|python3?|node)\s/m, /^#!\/(usr\/)?bin\/(env\s+)?(ba|z)?sh/]],
];

/** Share of characters that are code punctuation: prose has very little of it. */
function punctuationDensity(text: string): number {
  const marks = text.match(/[{}()[\];=<>|&$:]/g)?.length ?? 0;
  return marks / Math.max(1, text.length);
}

export function detectLanguage(text: string): Language | null {
  if (/^#!.*\bpython/.test(text)) return "python";
  for (const [lang, patterns] of RULES) {
    if (patterns.some((p) => p.test(text))) return lang;
  }
  // Fallback: only trust highlight.js when the text actually looks like code.
  if (text.length < 24 || punctuationDensity(text) < 0.025) return null;
  const auto = hljs.highlightAuto(text, Object.keys(LANGS));
  return auto.relevance >= 8 && auto.language ? (auto.language as Language) : null;
}

/** HTML for a code block. highlight.js escapes the input, so this is safe to inject. */
export function highlight(code: string, language: Language): string {
  return hljs.highlight(code, { language, ignoreIllegals: true }).value;
}
