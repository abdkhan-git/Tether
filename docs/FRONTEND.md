# Tether frontend

The desktop UI lives in [`app/`](../app): React 19 + TypeScript + Tailwind v4 + Motion, in a Tauri 2 shell (`app/src-tauri`, the `tether-app` crate). Inside the app it shows live data from `tetherd`; in a plain browser it runs against preview data, so the design can be worked on without a daemon.

```sh
cd app
npm install
npm run app        # the desktop app (builds tetherd first), against the daemon
npm run dev        # the browser preview at http://localhost:1420, sample data
npm test           # unit tests (Vitest)
npm run build      # typecheck + tests + production build of the page
npm run app:build  # release app and installer (stages tetherd beside it)
```

The app talks to the daemon for the `default` profile. Pass another with `npx tauri dev -- -- --profile left` (the second `--` hands the rest to the app), matching `tetherd --profile` and `tether --profile`. The first dev launch takes a while: Vite pre-bundles dependencies once.

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
- **This device:** its icon and default name follow the detected OS. Other devices do not report their OS yet, so they get a neutral computer icon and "Paired device".

User-agent sniffing is enough for this device inside the app too (WebView2 reports Windows, WKWebView macOS).

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

**First decide where it lives.** Most features belong on a page that exists (Pulse on Devices, the shelf in Transfers, Handoff in Workspaces) and are built as a section or panel of it, not a module of their own: the top navigation must not grow a tab per feature (owner, 2026-11-26). A new module is for a separate activity, like Notes.

Tether is a set of modules on one synced core. The top navigation, the search flyout and the Modules page are all generated from a registry, so a new module touches three places:

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

### Notes (`app/src/modules/notes`, `app/src/lib/notes`)

- **Editor:** a plain textarea driven by a `NoteSession` (`lib/notes/session.ts`), which batches typing (150 ms), keeps one edit in flight, carries incoming changes over what is unsent, and undoes only what was typed here (Ctrl+Z, Ctrl+Y). No DOM in the session: it is unit-tested against a stand-in daemon (`fake-daemon.ts`) with lag, lost answers, restarts and an input method. Deltas (`delta.ts`) mirror the Rust ones and are checked against cases the Rust tests generate.
- **Keys:** Enter continues a list or checklist (an empty item ends it), Tab and Shift+Tab indent, Ctrl+B and Ctrl+I wrap the selection, Ctrl+Enter ticks the checklist item under the caret.
- **Views:** Write, Read (Markdown from `lib/notes/markdown.ts`, rendered as React elements, never HTML; checkboxes tick the source; links go through `open_link`) and History (revisions by device, any version shown with who wrote it, restore as a new change).
- **Who wrote what:** a tint layer under the textarea (`.note-stack` in `index.css`), off until asked for. Tints `--author-0..5`: this device is a neutral grey, others muted teal, clay, rose, moss and stone (no green, red, blue, purple or gold).
- **Hidden characters** from other devices (escape codes, direction overrides, a lone carriage return) are shown as visible stand-ins of the same length (`visible()` in session.ts), so the textarea and the note line up one to one.
- **Secrets:** when the daemon finds a key or token in an edit, the page asks "Take it out" / "Keep it in the note" before anything is recorded.
- **Scratchpad:** a third window (label `scratchpad`, `app/src/scratchpad/`), Alt+Shift+N or the tray, floating and frameless, Esc puts it away. In the browser preview, `?scratchpad`.
- **Preview:** `data/notes.ts` holds sample notes in the browser, and another "device" types into one every few seconds, so merging and tints can be seen without the app.

### Planned modules

`app/src/modules/planned.ts` lists features that are designed but not built, in the order agreed in ROADMAP.md (Pulse, Compute, Drive, Rules, Timeline, then Vault and Spaces; Notes is built). Each needs its own threat model before it is built. Drop (the shared shelf) grows out of the Transfers module and Handoff out of Workspaces; both are shown under "Coming" inside the app.

## Data layer

`app/src/data/store.ts` is a zustand store of devices, clips, transfers, workspaces and log operations, plus a `Backend`: what the UI asks of whoever holds the data (revoke, pause, pair, join, pin, delete, older history, open a link, open at sign-in).

- **In the desktop app** (`main.tsx` checks `inDesktopApp`), `live.ts` swaps in the daemon backend. `daemon.ts` is the typed bridge: one function per Tauri command, and listeners for `tether://event` (new copies, device changes, pause) and `tether://link` (whether the daemon can be reached). `map.ts` turns daemon shapes into UI shapes and is unit-tested. Round trips are re-read every 3 s while the window is visible.
- **In a browser**, the preview backend and `startMockDaemon()` simulate peers copying text, transfer progress and latency jitter.

Modules carry `daemon: true` when the daemon implements what they show (Overview, Clipboard, Notes, Devices, Diagnostics). Inside the app the others (Transfers, Workspaces) are listed under "Coming" on the Modules page instead of showing sample data. Pin and "Delete everywhere" go through the daemon (signed operations merged on every device; see SECURITY.md, "Pins and deletes"). "Keep on this device" for an item already synced and "push to one device" are preview-only and hidden in the app.

### The desktop shell (`app/src-tauri`)

- **Commands:** `status`, `devices`, `history`, `pair`, `join`, `revoke`, `rename_device`, `set_paused`, `set_pinned`, `delete_item`, `send_item`, `copy_here`, `open_link`, `autostart`, `set_autostart`, `notifications`, `set_notifications`, `palette_shortcut`, `scratchpad_shortcut`, `show_scratchpad`, the notes commands (`notes`, `note_create`, `note_open`, `note_edit`, `note_meta`, `note_history`, `note_at`, `note_restore`), and the scratchpad window's two (`scratch_open`, `scratch_edit`), and nothing else. Adding one means adding it to `generate_handler!` in `main.rs`, to `COMMANDS` in `build.rs`, and its `allow-<name>` permission to the capability of each window that needs it (`capabilities/main.json`, `capabilities/palette.json`); without all three the page cannot call it.
- **Rules for the page** (SECURITY.md, "Desktop app"): no remote content, no inline `<style>` or `<script>` (the CSP blocks them), links only through `backend.openLink`, never `window.open` or `<a href>` to outside pages (the window refuses to navigate away from the app).
- **Window:** closing it hides it to the tray. Tray menu: Open Tether, Pause sync, Quit Tether. Its state always comes from the daemon.
- **Quick paste palette:** a second window (label `palette`, `app/src/palette/`), hidden until Alt+Shift+V; `main.tsx` mounts the palette instead of the app in that window (or with `?palette` in the browser preview). Its capability grants only status, devices, history and `copy_here`. Copying a history item here (palette, "Copy to this PC", search) goes through the daemon (`copy_here`), so it is not recorded as a new copy; derived text (formatted JSON, a link without trackers) still uses the browser clipboard, because it is new content.
- **Notifications:** sent from Rust (`notify.rs`) when a copy or a send arrives and the window is not focused. They never contain the copied text. Settings live in `%APPDATA%\Tether\app-<profile>.json`.
- **Daemon:** if none answers for the profile, the app starts the `tetherd` next to its executable with `--managed`, which stops when the app does. Its log is `%LOCALAPPDATA%\Tether\logs\tetherd-<profile>.log`.

## Known gaps

- Native window chrome for now: no Mica/vibrancy, no custom title bar, no global hotkey. The title bar still carries `data-tauri-drag-region` for later.
- File transfer and workspace capture are UI only; the daemon does not implement them yet.
- "Show in File Explorer" for copied paths is not offered: a network path from another device would leak the NTLM hash. It needs a local-paths-only check.
- Other devices' OS, names you choose for them, and their version vectors are not reported by the daemon yet.
- The JS bundle is about 245 KB gzipped. That's fine for a local app, but worth code-splitting per module if it grows.
- No automated UI tests yet; the shell was checked by hand through the WebView2 DevTools protocol.
