# Tether roadmap

**Tether makes all your computers behave like one computer.** Copy and paste is the first thing that runs on it. Everything else uses the same signed, replicated log and the same authenticated, encrypted connections, under the rules in [SECURITY.md](SECURITY.md).

Why not email, a chat app or a shared doc? Those need a deliberate action every time, route your data through someone else's server, and know nothing about your devices. Tether needs no action, keeps data on your devices, and knows which machines are yours, which are online, and what each one is doing.

## Flagships

Each one should be useful every day to one person with two or more computers, and each has a hard engineering core.

| # | Flagship | What you get | Engineering core | Capability |
| --- | --- | --- | --- | --- |
| F1 | **Smart clipboard** (exists in the UI) | Everything you copy, everywhere, recognised and formatted: code, JSON, links without trackers, tables, secrets kept local | Replicated log, CRDT history, content detection moved into the Rust core | `clipboard.*` |
| F2 | **One keyboard and mouse** | Move the cursor off your PC's screen edge and it drives the laptop beside it; the clipboard follows | Sub-10 ms input forwarding, OS input capture and injection, screen-edge handover without lost keys | `input.control` |
| F3 | **Localhost bridge** | `localhost:3000` on your desktop opens on your laptop, without exposing it to the network or the internet | Stream multiplexing over QUIC, per-port grants, backpressure | `ports.connect` |
| F4 | **Files, anywhere** | Drag a file to a device; huge files resume after any interruption; later, folders from every device show up in one place and download only when opened | Content-defined chunking, BLAKE3, parallel fetch from several peers, on-demand reads | `files.*` |
| F5 | **Search everything** | One search box over clipboard history, files and recent activity on every device | Indexes stay on each device; queries go to every peer at once with a latency budget, and partial results are merged when some peers are slow | `search.query` |
| F6 | **AI that knows your devices** | Ask an AI assistant "what did I copy on my desktop an hour ago?" or "run the tests on my desktop", using the standard MCP connector | Scoped read-only access by default, human confirmation for actions, every request in the audit log | `agent.read`, `agent.act` |
| F7 | **Pick up where you left off** | Hand your open tabs, files at the cursor position, and terminal folders to another machine | Field-level merge of workspace state (Automerge); browser extension | `workspace.*` |
| F8 | **Run it on the big machine** | `tether run test --on desktop` runs a task the desktop has defined and streams the output back; Ctrl+C works | Task definitions stay on the target, output streaming, cancellation propagation | `tasks.run` |
| F9 | **Settings that follow you** | Editor settings, git config, terminal profiles and dotfiles in sync, conflicts merged; secrets go through Vault and are never written to disk in plain text | CRDT merge of structured files, per-file rules | `config.sync` |
| F10 | **Device awareness** | Find my laptop (make it ring), battery and storage at a glance, Do Not Disturb on one device applies to all, a download on any device appears in one list | Presence, small state in the log | `presence.*` |
| F11 | **Live scratchpad** (added 2026-11-23) | Notes you type into on any of your computers, merged keystroke by keystroke, offline included | A sequence CRDT for text (RGA/Fugue family) in `tether-core`, batched into signed operations; the simulator proves convergence and no interleaving of concurrent words | `notes.*` |

Smaller modules that ride on the same core: **Vault** (move `.env` files and API keys, auto-expiring), **Drop folders** (screenshots go straight to the laptop), **Remote actions** (lock or sleep another device), and, with mobile, **notification mirroring** and **phone camera as a scanner**.

## How each flagship stays safe

| Flagship | Risk if abused | Design that contains it |
| --- | --- | --- |
| F1 clipboard | Pasting a malicious command; leaking secrets | Secrets never sync; command-like text from a peer goes to history, not straight to the system clipboard |
| F2 input | A compromised peer types on your machine | Off by default; accepted on the controlled device each session; on-screen indicator; local hotkey ends control instantly; LAN only to begin with |
| F3 ports | Reaching services you didn't mean to share | Nothing shared by default; only ports shared locally; never exposed to anything but paired devices |
| F4 files | Malicious files, path tricks (`..\..\`) | Quarantine folder; never auto-opened; file names sanitised; hashes verified per chunk |
| F5 search | Leaking private data to another device | Opt-in per device; sensitive items excluded from indexes |
| F6 AI | Prompt injection makes an agent do harm | Read-only by default; actions need a person to confirm on the affected device; full audit log |
| F7 workspace | Opening a malicious URL or file | Restoring is always a local user action; links shown before opening |
| F8 tasks | Remote code execution | No arbitrary commands, ever: only tasks the target defines locally, confirmed on the target |
| F9 config | Overwriting settings with something harmful | Opt-in per file; history and one-click rollback; secrets only through Vault |
| F10 presence | Location or activity tracking | Shared only among your own devices; nothing leaves them |
| F11 scratchpad | Hidden or rewritten notes; a huge paste that bloats every device | Every edit is a signed operation from a member; size limits per note and per operation batch; secrets detection on paste, like the clipboard |

## What you can do in the app

Each feature is something the user does in the Tether window or tray, not only something to look at. Status as of 2026-11-23.

| Area | What the user does | Feature | Status |
| --- | --- | --- | --- |
| Clipboard | Browse, search and copy back anything copied on any device; content shown the way it reads (JSON, code, links without trackers, tables, colours) | F1 | Done |
| Clipboard | Pin or delete an item on every device | F1 · P2.1 | Done |
| Clipboard | Send one history item straight to one device's clipboard | F1 quick win | Done |
| Anywhere | Press Alt+Shift+V in any app, search history from every device, press Enter to put it on the clipboard (quick paste palette) | F1 quick win | Done |
| Devices | Pair with a six-digit code, revoke, see live round-trip time | P0/P1 | Done |
| Devices | Rename a device; the name follows on every device | P2.3 quick win | Done |
| Tray | Pause sync, open at sign-in, notifications when a copy arrives from another device | P1 + quick win | Done |
| Notes | Write notes that merge live across devices, offline included | F11 | Next (first big feature) |
| Transfers | Drag a file onto a device; watch progress; resume after any interruption; open received files from a quarantine folder | F4 | Planned |
| Tasks | Run a task another device has defined ("run tests on the desktop"), watch output live, cancel | F8 | Planned |
| Network | Share a local port (`localhost:3000`) with your other devices | F3 | Planned |
| Desk | Use one keyboard and mouse across both screens | F2 | Planned (needs both machines side by side) |
| Search | One box over clipboard, notes and files on every device | F5 | Later |
| Workspaces | Hand open tabs, files and terminal folders to another device | F7 | Later |
| Assistant | Ask an AI about your devices; it acts only with your confirmation | F6 | Later (deliberately after the features above) |

## Phases

Each phase ends with something usable and something measurable.

| Phase | Ships | Done when |
| --- | --- | --- |
| **P0 · Security foundation** ✅ | Device identity keys in the OS keychain; pairing with a short code (SPAKE2 bound to the TLS channel); mutual TLS with pinned keys; signed operations; membership and revocation in the log; limits on every decoder and peer; property-based fuzzing of the decoder; `cargo deny` and `npm audit` in CI; content policy (secrets, pastejacking); durable log re-verified on load | **Done 2026-11-23.** V1 to V5 closed, each covered by unit, end-to-end or simulator tests (SECURITY.md). Deferred: capability grants land with the first feature that needs one (F8 tasks, P3); `cargo-fuzz` targets (nightly) alongside the stable property tests. |
| **P1 · Real app** (all built; the two-machine check waits for a second device) | Tauri shell and tray, open at sign-in; authenticated local channel between UI and daemon; the UI's sample data replaced with live data; OS clipboard change events instead of polling; the log saved to SQLite; reconnect with backoff; the `tether` CLI | Copy on one computer, see it in the Tether window on the other, paired and encrypted, after a reboot of either |
| **P2 · Core correctness and quick wins** (in progress: pins and deletes done 2026-11-23) | Merge rules for pins, deletes and settings (device names); the quick wins: notifications, send to one device, quick paste palette; compaction; content detection and sensitivity in Rust; simulator faults for crashes, clock skew and revocation mid-sync | 10,000 seeds pass with the new invariants; p50/p99 clipboard latency between the owner's two Windows machines published in the README |
| **P3 · Beyond the clipboard** (re-ordered 2026-11-23) | F11 live scratchpad; F4 file send with resume; F8 run it on the big machine | Concurrent offline edits converge with no interleaving (simulator); a 10 GB file resumes after a cable pull with no chunk re-sent; a remote task streams output and cancels cleanly |
| **P4 · One desk** | F2 one keyboard and mouse; F3 localhost bridge; F4 stage 2 (folders from every device, fetched on demand) | Measured input latency and handover between the two machines |
| **P5 · Beyond the home network** | Signaling Worker, hole punching, encrypted relay | Clipboard and files work between two different networks |
| **P6 · Continuity and search** | F5 search everything; F7 workspace handoff; F9 settings sync; F10 device awareness; F1 per-app policy rules; Vault and Drop folders | Resuming a workspace takes one click; a search answers from every device within its latency budget |
| **P7 · Intelligence** | F6 AI context over MCP | An agent answers questions about your devices with no data leaving them, and every action is confirmed and logged |
| **P8 · Mobile and sharing** | Android, then iOS; Spaces shared with other people | A phone joins with the same pairing flow |

## Decisions to make along the way

- **Sign-in with an account (decided 2026-11-23):** not now. Possibly later, and only to help a new device *find* yours; an existing device must still approve it. Trust always comes from pairing and device keys, never from the account, because an account that can add devices would let whoever controls it (or the account provider) read your data.
- **Input sharing** ships Windows ↔ Windows first (the owner's desktop and laptop). macOS needs the Accessibility permission, and Wayland needs a portal; both follow, and stay marked unverified until run on real machines.
- **AI integration** follows the MCP specification current at the time; capability names above are Tether's own.
