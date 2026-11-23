// Formatting for code and data. Real formatters (Prettier, sql-formatter) are
// loaded on first use so they cost nothing until someone copies that language.
// Languages without an in-browser formatter get their indentation repaired.

import type { Language } from "./language";

export interface Formatted {
  text: string;
  /** What did the work, shown to the user: "Prettier", "Fixed indentation". */
  tool: string;
}

const PRETTIER: Partial<Record<Language, string>> = {
  javascript: "babel",
  typescript: "typescript",
  css: "css",
  markdown: "markdown",
  yaml: "yaml",
};

async function prettier(text: string, parser: string): Promise<string> {
  const [core, babel, estree, ts, postcss, md, yaml] = await Promise.all([
    import("prettier/standalone"),
    import("prettier/plugins/babel"),
    import("prettier/plugins/estree"),
    import("prettier/plugins/typescript"),
    import("prettier/plugins/postcss"),
    import("prettier/plugins/markdown"),
    import("prettier/plugins/yaml"),
  ]);
  return core.format(text, { parser, plugins: [babel, estree, ts, postcss, md, yaml], printWidth: 90 });
}

/** Returns a formatted copy, or null when nothing would change or formatting failed. */
export async function formatCode(text: string, language: Language): Promise<Formatted | null> {
  try {
    let out: string;
    let tool: string;
    if (language === "json") {
      out = JSON.stringify(JSON.parse(text), null, 2);
      tool = "Pretty-printed";
    } else if (language === "sql") {
      const { format } = await import("sql-formatter");
      // PostgreSQL's grammar keeps `now()` intact; generic SQL is the fallback.
      try {
        out = format(text, { language: "postgresql", keywordCase: "upper", tabWidth: 2 });
      } catch {
        out = format(text, { language: "sql", keywordCase: "upper", tabWidth: 2 });
      }
      tool = "sql-formatter";
    } else if (PRETTIER[language]) {
      out = await prettier(text, PRETTIER[language]!);
      tool = "Prettier";
    } else {
      out = repairIndent(text);
      tool = "Fixed indentation";
    }
    out = out.replace(/\s+$/, "");
    return out !== text.replace(/\s+$/, "") ? { text: out, tool } : null;
  } catch {
    return null;
  }
}

const leading = (line: string) => /^[ \t]*/.exec(line)![0];
const width = (ws: string, tab: number) => [...ws].reduce((n, c) => n + (c === "\t" ? tab : 1), 0);

/** The indentation step the text uses: tab, 2 or 4 spaces. */
export function indentUnit(lines: string[]): string {
  if (lines.some((l) => l.startsWith("\t"))) return "\t";
  const widths = lines.filter((l) => l.trim()).map((l) => leading(l).length).filter((w) => w > 0);
  if (!widths.length) return "    ";
  return widths.every((w) => w % 4 === 0) ? "    " : "  ";
}

/**
 * Repairs the indentation of a copied snippet:
 * - removes the indentation shared by every line (copied from deep inside a file);
 * - fixes the common "first line has no indent" selection, where the rest of
 *   the block is still indented relative to where the copy started;
 * - trims trailing whitespace and surrounding blank lines.
 */
export function repairIndent(text: string): string {
  let lines = text.replace(/\r\n/g, "\n").split("\n").map((l) => l.replace(/[ \t]+$/, ""));
  while (lines.length && !lines[0].trim()) lines.shift();
  while (lines.length && !lines[lines.length - 1].trim()) lines.pop();
  if (lines.length < 2) return lines.join("\n");

  const unit = indentUnit(lines);
  const tab = unit === "\t" ? 4 : unit.length;
  const widthOf = (l: string) => width(leading(l), tab);
  const body = lines.slice(1).filter((l) => l.trim());
  const first = lines[0];
  const bodyMin = body.length ? Math.min(...body.map(widthOf)) : 0;

  let shift: number;
  if (widthOf(first) === 0 && bodyMin > 0) {
    // The selection started mid-line, so the first line lost its indent. If it
    // opens a block, a closing line (`}`, `end`) sits at the opener's original
    // indent; without one, the body is one level deeper than the opener.
    const opens = /[{([:]\s*$|=>\s*$|\b(do|then)\s*$/.test(first);
    const last = body[body.length - 1];
    const closes = /^\s*([}\])]|end\b|fi\b|done\b|esac\b)/.test(last);
    shift = opens ? (closes ? widthOf(last) : Math.max(0, bodyMin - tab)) : bodyMin;
    lines = [first, ...lines.slice(1).map((l) => dedent(l, shift, tab))];
  } else {
    shift = Math.min(...lines.filter((l) => l.trim()).map(widthOf));
    lines = lines.map((l) => dedent(l, shift, tab));
  }
  return lines.join("\n");
}

function dedent(line: string, amount: number, tab: number): string {
  if (!line.trim()) return "";
  let removed = 0;
  let i = 0;
  while (i < line.length && removed < amount && (line[i] === " " || line[i] === "\t")) {
    removed += line[i] === "\t" ? tab : 1;
    i++;
  }
  return line.slice(i);
}
