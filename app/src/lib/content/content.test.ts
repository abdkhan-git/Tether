import { describe, expect, it } from "vitest";
import { detect } from "./detect";
import { formatCode, repairIndent } from "./format";
import { evaluate, joinLines, parseColor, parseLink, parsePath, textStats, titleCase, toHex, toHslString } from "./parsers";

const kind = (t: string) => detect(t).kind;
const jwt = (payload: object) => {
  const b64 = (o: object) => Buffer.from(JSON.stringify(o)).toString("base64url");
  return `${b64({ alg: "HS256", typ: "JWT" })}.${b64(payload)}.c2lnbmF0dXJlLXNpZ25hdHVyZQ`;
};

describe("detect: kinds", () => {
  it("JSON objects and arrays, with shape facts", () => {
    const c = detect('{"a":1,"b":{"c":[1,2,{"d":true}]}}');
    expect(c.kind).toBe("json");
    if (c.kind === "json") {
      expect(c.entries).toBe(2);
      expect(c.depth).toBe(4);
    }
    expect(kind("[1, 2, 3]")).toBe("json");
    expect(kind("{not json}")).not.toBe("json");
  });

  it("links, recognising GitHub and stripping trackers", () => {
    const c = detect("https://github.com/rust-lang/rust/pull/12345?utm_source=x&tab=files");
    expect(c.kind).toBe("link");
    if (c.kind === "link") {
      expect(c.link.title).toBe("Pull request #12345 · rust-lang/rust");
      expect(c.link.tracking).toEqual(["utm_source"]);
      expect(c.link.clean).toBe("https://github.com/rust-lang/rust/pull/12345?tab=files");
    }
    expect(kind("www.example.com/page")).toBe("link");
    expect(kind("not a link at all")).toBe("text");
  });

  it("keeps a non-YouTube `si` parameter", () => {
    expect(parseLink("https://example.com/?si=keep")!.clean).toBe("https://example.com/?si=keep");
    expect(parseLink("https://youtu.be/abc?si=drop")!.clean).toBe("https://youtu.be/abc");
  });

  it("colours in hex, rgb and hsl", () => {
    expect(kind("#1e90ff")).toBe("color");
    expect(kind("rgb(30, 144, 255)")).toBe("color");
    expect(kind("hsl(210, 100%, 56%)")).toBe("color");
    expect(toHex(parseColor("#abc")!)).toBe("#AABBCC");
    expect(toHslString(parseColor("#ff0000")!)).toBe("hsl(0, 100%, 50%)");
    expect(parseColor("rgb(300, 0, 0)")).toBeNull();
  });

  it("JWTs are decoded and always sensitive", () => {
    const c = detect(jwt({ sub: "42", exp: 4_000_000_000 }));
    expect(c.kind).toBe("jwt");
    expect(c.sensitive).toBe(true);
    if (c.kind === "jwt") expect(c.jwt.payload.sub).toBe("42");
  });

  it("secrets by pattern, kept on the device", () => {
    for (const [text, name] of [
      ["AKIAIOSFODNN7EXAMPLE", "AWS access key"],
      ["ghp_" + "a".repeat(36), "GitHub token"],
      ["postgres://admin:s3cret@db.internal:5432/app", "Connection string"],
      ["-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----", "Private key"],
      ["Xk9#mQ2$vL7pR4", "Password"],
    ]) {
      const c = detect(text);
      expect(c.kind, text).toBe("secret");
      expect(c.sensitive).toBe(true);
      expect(c.label).toBe(name);
    }
  });

  it("names secrets naturally mid-sentence", () => {
    const c = detect("AKIAIOSFODNN7EXAMPLE");
    if (c.kind === "secret") expect(c.secret.noun).toBe("an AWS access key");
  });

  it("does not call ordinary identifiers or slugs passwords", () => {
    for (const t of ["useLatencyHistory", "my-blog-post-2024", "snake_case_name", "README.md"]) expect(kind(t), t).not.toBe("secret");
  });

  it("timestamps: unix seconds, milliseconds and ISO", () => {
    expect(kind("1700000000")).toBe("time");
    expect(kind("1700000000000")).toBe("time");
    expect(kind("2026-11-23T09:30:00Z")).toBe("time");
    expect(kind("2026-11-23")).toBe("time");
    expect(kind("1234567")).not.toBe("time");
  });

  it("file paths, with WSL conversion", () => {
    const p = parsePath("C:\\Users\\me\\Documents\\report final.pdf")!;
    expect(p.style).toBe("windows");
    expect(p.name).toBe("report final.pdf");
    expect(p.extension).toBe("pdf");
    expect(p.alternate).toEqual({ label: "WSL path", value: "/mnt/c/Users/me/Documents/report final.pdf" });
    expect(parsePath("/mnt/d/projects/x.rs")!.alternate!.value).toBe("D:\\projects\\x.rs");
    expect(kind("~/code/tether/Cargo.toml")).toBe("path");
    expect(kind("\\\\fileserver\\share\\q4.xlsx")).toBe("path");
  });

  it("path matching stays fast on hostile input", () => {
    const start = performance.now();
    parsePath("C:\\" + "a".repeat(900) + "<");
    expect(performance.now() - start).toBeLessThan(50);
  });

  it("arithmetic, but not phone numbers or dates", () => {
    const c = detect("(12 + 8) * 3 / 4");
    expect(c.kind).toBe("math");
    if (c.kind === "math") expect(c.result).toBe(15);
    expect(evaluate("2 ^ 3 ^ 2")).toBe(512);
    expect(evaluate("-(4 - 6) * 1.5")).toBe(3);
    expect(evaluate("1 +")).toBeNull();
    expect(kind("555-1234")).not.toBe("math");
    expect(kind("42")).toBe("text");
  });

  it("tables from spreadsheets and CSV, with quoted fields", () => {
    const c = detect('name,role,city\n"Khan, A",eng,Toronto\nLee,pm,"New York"');
    expect(c.kind).toBe("table");
    if (c.kind === "table") expect(c.table.rows[1]).toEqual(["Khan, A", "eng", "Toronto"]);
    expect(kind("a\tb\n1\t2")).toBe("table");
  });

  it("email and UUID", () => {
    expect(kind("someone@example.com")).toBe("email");
    const u = detect("123e4567-e89b-42d3-a456-426614174000");
    expect(u.kind).toBe("uuid");
    if (u.kind === "uuid") expect(u.version).toBe(4);
  });
});

describe("detect: code languages", () => {
  const cases: [string, string][] = [
    ["rust", "fn order_key(&self) -> (Timestamp, DeviceId) {\n    (self.ts, self.device)\n}"],
    ["python", "def fib(n: int) -> int:\n    return n if n < 2 else fib(n - 1) + fib(n - 2)"],
    ["go", "package main\n\nfunc main() {\n\tx := 1\n}"],
    ["typescript", "interface Device {\n  id: string;\n  seq: number;\n}"],
    ["javascript", "const total = items.reduce((a, b) => a + b, 0);\nconsole.log(total);"],
    ["sql", "select device, max(seq) from ops group by device;"],
    ["bash", "cargo run --release -p tether-sim -- --seeds 10000"],
    ["powershell", "Get-ChildItem -Recurse | Where-Object { $_.Length -gt 1MB }"],
    ["csharp", "using System;\nnamespace Demo { public class A { } }"],
    ["xml", '<div class="card"><span>hi</span></div>'],
    ["css", ".card {\n  padding: 12px;\n  color: red;\n}"],
    ["dockerfile", "FROM rust:1.80\nRUN cargo build --release"],
  ];
  for (const [lang, src] of cases) {
    it(lang, () => {
      const c = detect(src);
      expect(c.kind, src).toBe("code");
      if (c.kind === "code") expect(c.language).toBe(lang);
    });
  }

  it("leaves prose alone", () => {
    expect(kind("Meeting notes: move relay work to stage 3, ship LAN clipboard first.")).toBe("text");
    expect(kind("Remember to buy milk, eggs and bread on the way home.")).toBe("text");
  });
});

describe("formatting", () => {
  it("pretty-prints minified JSON", async () => {
    const f = await formatCode('{"a":1,"b":[1,2]}', "json");
    expect(f?.text).toBe('{\n  "a": 1,\n  "b": [\n    1,\n    2\n  ]\n}');
  });

  it("formats SQL", async () => {
    const f = await formatCode("select a,b from t where x=1", "sql");
    expect(f?.text).toContain("SELECT\n");
    expect(f?.tool).toBe("sql-formatter");
  });

  it("formats TypeScript with Prettier", async () => {
    const f = await formatCode("const x={a:1,b:2}", "typescript");
    expect(f?.text).toBe("const x = { a: 1, b: 2 };");
  });

  it("returns null when already tidy, or when the code does not parse", async () => {
    expect(await formatCode('{\n  "a": 1\n}', "json")).toBeNull();
    expect(await formatCode("const = = ;", "javascript")).toBeNull();
  });

  it("repairs a selection copied from mid-line", () => {
    const copied = "fn main() {\n            let x = 1;\n            println!(\"{x}\");\n        }";
    expect(repairIndent(copied)).toBe("fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}");
  });

  it("repairs a mid-line selection that stops before the closing line", () => {
    expect(repairIndent("if ready:\n            go()\n            log()")).toBe("if ready:\n    go()\n    log()");
  });

  it("removes indentation shared by every line", () => {
    expect(repairIndent("        if x:\n            y()\n")).toBe("if x:\n    y()");
  });
});

describe("plain text helpers", () => {
  it("spots hard-wrapped text and joins it", () => {
    const pdf = "Tether keeps an append-only log on each\ndevice and replicates it to every peer\nso that copies converge after partitions.\n\nSecond paragraph here.";
    expect(textStats(pdf).hardWrapped).toBe(true);
    expect(joinLines(pdf)).toBe("Tether keeps an append-only log on each device and replicates it to every peer so that copies converge after partitions.\n\nSecond paragraph here.");
    expect(textStats("One line.\nAnother line.").hardWrapped).toBe(false);
  });

  it("title-cases without capitalising small words", () => {
    expect(titleCase("the art of computer programming")).toBe("The Art of Computer Programming");
  });
});
