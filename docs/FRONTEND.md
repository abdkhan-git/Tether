# Tether frontend

The desktop UI lives in [`app/`](../app): React 19 + TypeScript + Tailwind v4 + Motion. It is the UI half of the Tauri 2 app described in [DESIGN.md](DESIGN.md). Today it runs in a browser against preview data; the Tauri shell and the daemon client come next.

```sh
cd app
npm install
npm run dev        # http://localhost:1420
npm test           # unit tests (Vitest)
npm run build      # typecheck + tests + production build
```

## Design language

A calm, professional desktop app in the spirit of Apple's own software, and at home on Windows, where the owner works.

| Aspect | Decision |
| --- | --- |
| Palette | Neutral graphite (`#0f0f11` dark, `#f5f5f7` light). Colour is a signal, never decoration: system green means connected, live or done; red means destructive. Selection, hover and primary buttons are neutral. No gold, no purple or blue. |
| Depth | Surfaces step up from background to surface to glass, separated by hairlines, with a faint neutral sheen at the top of the window. Cards catch a barely visible neutral highlight under the cursor. |
| Type | Geist throughout (semibold, tight tracking for titles and figures, like SF Pro Display), IBM Plex Mono for code, IDs and speeds. |
| Navigation | A floating glass pill bar across the top (React Bits `PillNav`). There is no sidebar. |
| Pages | A small grey eyebrow, a bold title, a one-line subtitle, at most one primary action (an inverted pill). |
| Code | Syntax colours follow Xcode's default themes (light and dark), defined as `--syn-*` tokens. |
| Icons | Hugeicons (`@hugeicons/core-free-icons`), 1.5 px stroke, all named in `components/Icon.tsx`. |
| Motion | Springs everywhere (around `duration: 0.5, bounce: 0.15`). Pages cross-fade with a slight blur; lists animate by layout; selection highlights glide. Respects `prefers-reduced-motion`. |

Tokens are in `app/src/index.css`. Libraries that need hex values (shaders, contrast maths) read them from `palette` in `app/src/components/ui.tsx`; keep the two in step.

## Platform awareness

`app/src/lib/platform.ts` detects the OS and the UI follows it:

- **Search:** a field in the title bar that opens a results flyout below it, as in Windows 11. It is focused with `Ctrl+F` (`⌘F` on a Mac) or `/`. There is no Mac-style command palette.
- **Wording:** "Copy to this PC", "Show in File Explorer"; on macOS, "this Mac" and "Finder".
- **Notifications:** bottom-right, where Windows shows its own.
- **This device:** its icon and default name follow the detected OS.

Inside Tauri, the OS plugin can replace the user-agent sniffing.

## Content-aware clipboard

The clipboard does not just store text: it works out what the text *is* and shows it accordingly. The engine lives in `app/src/lib/content/` and is pure TypeScript with unit tests (`content.test.ts`), so it can later move into the Rust core if the daemon needs it.

| Detected | Shown as | Extras |
| --- | --- | --- |
| JSON | Pretty-printed, syntax-highlighted, line numbers | Shape summary (keys, depth, size); formatted / original toggle |
| Code (18 languages) | Highlighted in the detected language | Real formatting for JS, TS, CSS, Markdown, YAML (Prettier) and SQL (sql-formatter); other languages get their indentation repaired, including the "copied from mid-line" case |
| Link | Title for known sites (GitHub PRs, issues, files; YouTube), query parameters | Trackers (`utm_*`, `fbclid`, `gclid`, ...) flagged; copy without trackers |
| Colour | Swatch | HEX, RGB, HSL conversions; WCAG contrast on white and black |
| Date / timestamp | Local time and relative time | Unix seconds, milliseconds, ISO 8601, UTC |
| Table (CSV, TSV, spreadsheet copy) | A real table | Copy as Markdown or JSON |
| File path | Breadcrumb | Show in File Explorer / Finder; Windows ⇄ WSL path conversion |
| JWT | Decoded header and payload | Expiry status; always kept on this device |
| Secrets (AWS, GitHub, OpenAI, Anthropic, Stripe, Slack, Google keys, private keys, connection strings with passwords, password-like tokens) | Masked until revealed | Automatically marked sensitive, so they never sync |
| Arithmetic | The answer | Copy result |
| UUID, email | Clean value | Case and format variants; write an email |
| Plain text | Readable text | Join lines broken mid-sentence (PDF copies); case conversions; word count |

Detection order and rules are in `detect.ts`; language rules are in `language.ts`. Heuristics run first, with highlight.js auto-detection as a fallback only for text that looks like code. Formatters are loaded lazily, so they cost nothing until that language is copied. Results are cached per clip.

## Component libraries

Polished interaction components are vendored from React Bits and Magic UI into `app/src/components/rb` and `app/src/components/magic`. See [VENDORED.md](../app/src/components/VENDORED.md) for what each one does, where it came from and its licence. React Bits is MIT + Commons Clause: fine to ship inside Tether, not to resell on its own.

Pages use the wrappers in `components/ui.tsx` (`Card`, `PrimaryAction`, `IconButton`, `Segmented`, `Switch`, `Figure`, ...) rather than the vendored files directly, so the look stays consistent and the vendored code can be refreshed from upstream.

## Adding a feature (module)

Tether is a set of modules on one synced core. The top navigation, the search flyout and the Modules page are all generated from a registry, so a new feature touches three places:

1. **Operation type.** Add the variant to `OpKind` in `crates/tether-core/src/op.rs`, with its merge rule, and extend the simulator's invariants (CLAUDE.md: every sync change keeps the simulator green).
2. **Module file.** Create `app/src/modules/<name>.tsx` exporting a `TetherModule`:

   ```tsx
   export const vaultModule: TetherModule = {
     id: "vault",
     label: "Vault",
     icon: icons.key,               // from components/Icon.tsx
     group: "Sync",
     summary: "One line for the Modules page and search.",
     status: "preview",             // stable | preview | planned
     view: VaultView,               // starts with <PageHeader eyebrow title subtitle />
     commands: () => [...],         // optional search-flyout actions
   };
   ```

3. **Registry.** Add it to `liveModules` in `app/src/modules/registry.ts`, and delete its entry from `planned.ts` if it was listed there.

Users can turn any non-core module off from the Modules page (the grid button in the title bar); the choice is stored locally.

### Planned modules

`app/src/modules/planned.ts` lists features that are designed but not built. Spaces is on the roadmap (Stage 5). Vault, Drop folders and Remote actions are **proposals**: they are not in DESIGN.md yet and each needs a design pass. Remote actions in particular needs a threat model, since it lets one device trigger behaviour on another.

## Data layer

`app/src/data/store.ts` is a zustand store of devices, clips, transfers, workspaces and log operations. `startMockDaemon()` simulates peers copying text, transfer progress and latency jitter. Every figure on screen is preview data until the daemon is connected, and the UI says so where it matters (pairing, workspaces).

To connect the daemon, replace the mock actions with Tauri commands and feed daemon events into the same store. Types in `app/src/data/types.ts` mirror `tether-core` (`OpRow` is `(device, seq, ts, kind)`).

## Known gaps

- No Tauri shell yet (window chrome, tray, Mica/vibrancy, global hotkey). The title bar already carries `data-tauri-drag-region`.
- Pairing, file transfer and workspace capture are UI only; the daemon does not implement them yet.
- The JS bundle is about 205 KB gzipped. That's fine for a local app, but worth code-splitting per module if it grows.
- No automated UI tests yet.
