# Tether roadmap

**Tether makes all your computers behave like one computer.** Copy and paste was the first thing that ran on it. Everything else uses the same signed, replicated log and the same authenticated, encrypted connections, under the rules in [SECURITY.md](SECURITY.md).

Why not email, a chat app or a shared doc? Those need a deliberate action every time, route your data through someone else's server, and know nothing about your devices. Tether needs no action, keeps data on your devices, and knows which machines are yours, which are online, and what each one is doing.

## Flagships

Each one should be useful every day to one person with two or more computers, and each has a hard engineering core. The numbers are stable names (other docs refer to them), not the build order; the build order is under "Phases". Re-planned 2026-11-23 with the owner: F4 became the shared shelf, F8 became Compute, F10 became Pulse, and F12 to F15 were added.

| # | Flagship | What you get | Engineering core | Capability |
| --- | --- | --- | --- | --- |
| F1 | **Smart clipboard** (done) | Everything you copy, everywhere, recognised and formatted; pin, delete, send to one device, quick paste palette; secrets kept local | Replicated log, merge rules for pins/deletes/names, content policy | `clipboard.*` |
| F2 | **Flow: one keyboard and mouse** | Move the cursor off the desktop's screen edge and it drives the laptop; the clipboard follows; control is released if the link drops | Input over QUIC datagrams (never behind a file transfer), OS input capture and injection, screen arrangement and coordinate mapping, handover without lost keys | `input.control` |
| F3 | **Localhost bridge** | `localhost:3000` on the desktop opens on the laptop, without exposing it to the network | Stream multiplexing over QUIC, per-port grants, backpressure | `ports.connect` |
| F4 | **Drop: the shared shelf** (files, stage 1) | Drag a file, folder, screenshot, link or text onto a floating shelf; open the shelf on another computer and drag it out. No destination to choose. Pinned and expiring items, collections | Shelf metadata in the log; contents stored by BLAKE3 hash, chunked, fetched on demand from whichever device has them, resumable; garbage collection when items leave | `files.*` |
| F5 | **Search everything** | One search box over clipboard, notes, the shelf and files on every device | Indexes stay on each device; queries go to every peer with a latency budget; partial results merged when a peer is slow | `search.query` |
| F6 | **AI that knows your devices** | Ask an assistant about your devices; it acts only with your confirmation | MCP server over the log; read-only by default; every action confirmed on the affected device and logged | `agent.read`, `agent.act` |
| F7 | **Handoff: application continuity** | Resume supported apps at the same logical state on another computer: VS Code files and cursor, browser tabs, PDF page, terminal folder, the open note | Adapter plugins with versioned state schemas, distributed checkpoints, cross-platform path mapping | `workspace.*` |
| F8 | **Compute: borrow processing power** | From the laptop, run a supported job (video conversion, compression, code indexing, later local AI) on the desktop; Tether picks the best device; watch progress, cancel | Job queue and scheduler using Pulse's resource data, worker pools, backpressure, cancellation, retries and failure recovery; jobs predefined on the worker, sandboxed, resource-limited | `tasks.run` |
| F9 | **Settings that follow you** | Editor settings, git config, terminal profiles and dotfiles in sync, conflicts merged | Merge of structured files, per-file rules | `config.sync` |
| F10 | **Pulse: device dashboard** | See every computer's status: online, RTT, CPU, memory, storage, battery, active transfers and jobs; Wake-on-LAN; "task finished" and "low storage" notices | Heartbeats and failure detection, streaming telemetry with adaptive sampling, small time series; the resource discovery Compute schedules with | `presence.*` |
| F11 | **Notes: live distributed notepad** | Notes that merge character by character across devices, offline for hours included; Markdown, code blocks, checklists; notebooks and pinned notes; version history and restore; see which device wrote what; a floating scratchpad on a shortcut | A sequence CRDT for text in `tether-core`, operations batched per burst of typing, causal delivery, compaction; the simulator proves convergence with no lost or interleaved edits | `notes.*` |
| F12 | **Drive: shared folders on demand** (files, stage 2) | Folders you share appear on your other computers (in File Explorer, later); files download when opened and stay cached | Read-through cache with eviction, chunk streaming, prefetching; Windows Cloud Files API (what OneDrive uses), FUSE/File Provider elsewhere; read-only first | `files.*` |
| F13 | **Rules: cross-device automation** | "When I take a screenshot, put it on the shelf"; "when a job finishes, notify the device I'm using" | Event-driven rules, durable execution, deduplication so that two devices seeing the same event run an action once (claims in the log), retries | `rules.*` |
| F14 | **Timeline** | Scroll back through what happened on your computers (copies, notes, shelf items, jobs) and restore from it | Event sourcing over the log, temporal queries, retention that coexists with compaction | (read only) |
| F15 | **Relay: deliver through other devices** | Send to a computer that is off; another of your devices holds it and delivers it later | Store-and-forward of large payloads, encrypted to the recipient, delivery acknowledgements, idempotency | `files.*` |

Smaller modules on the same core: **Vault** (move `.env` files and API keys, auto-expiring), **Remote actions** (lock or sleep another device), a **whiteboard** in Notes (shapes and strokes need their own merge rules), and, with mobile, notification mirroring.

## How each flagship stays safe

| Flagship | Risk if abused | Design that contains it |
| --- | --- | --- |
| F1 clipboard | Pasting a malicious command; leaking secrets | Secrets never sync; command-like text from a peer goes to history, not the clipboard; pins/deletes/sends name items by content hash |
| F2 Flow | A compromised peer types on your machine | Off by default; accepted on the controlled device each session; on-screen indicator; local hotkey ends control instantly; released when the link drops |
| F3 ports | Reaching services you didn't mean to share | Nothing shared by default; only ports shared locally; only paired devices |
| F4 Drop | Malicious files, path tricks (`..\..\`), a full disk | Items land in a quarantine folder, never auto-opened; names sanitised; every chunk verified by hash; size quotas and expiry |
| F5 search | Leaking private data to another device | Opt-in per device; sensitive items excluded from indexes |
| F6 AI | Prompt injection makes an agent do harm | Read-only by default; actions confirmed by a person on the affected device; audit log |
| F7 Handoff | Opening a malicious URL or file | Restoring is a local user action; links and paths shown before opening |
| F8 Compute | Remote code execution; a crafted input exploiting the worker's tool (a media converter) | No arbitrary commands: only job types defined on the worker and confirmed there; inputs only from your devices; jobs sandboxed with CPU, memory, time and disk limits |
| F9 config | Overwriting settings with something harmful | Opt-in per file; history and rollback; secrets only through Vault |
| F10 Pulse | Activity tracking | Shared only among your own devices; Wake-on-LAN only to paired devices on the LAN |
| F11 Notes | Hidden or rewritten notes; a huge paste that bloats every device | Every edit is a signed operation from a member; size limits per note and per batch; secret detection on paste |
| F12 Drive | Exposing more than you meant; tampered files | Only folders shared locally, read-only first; content verified by hash |
| F13 Rules | Automation as a path to remote execution | Rules run actions only on the device that defines them, from an allow-list; one execution per event |
| F14 Timeline | A record of everything you did | Built only from Tether's own events and enabled integrations; retention limits; no screen recording |
| F15 Relay | An intermediate device reading what it holds | Payloads encrypted to the recipient device; the holder sees ciphertext only |

## What you can do in the app

Each feature is something the user does in the Tether window or tray, not only something to look at. Status as of 2026-11-23.

| Area | What the user does | Feature | Status |
| --- | --- | --- | --- |
| Clipboard | Browse, search and copy back anything copied on any device; content shown the way it reads | F1 | Done |
| Clipboard | Pin or delete an item on every device; send an item to one device | F1 | Done |
| Anywhere | Alt+Shift+V in any app: search history from every device, Enter to paste it here | F1 | Done |
| Devices | Pair with a six-digit code, rename, revoke, see live round-trip time | P0/P1 | Done |
| Tray | Pause sync, open at sign-in, notifications | P1 | Done |
| Notes | Write notes that merge live across devices, offline included; Markdown and checklists; notebooks, pins, trash; history and restore; who wrote what; floating scratchpad (Alt+Shift+N) | F11 | Done (single machine; the two-machine check waits for the laptop) |
| Pulse | See every computer's health and activity; wake a sleeping one | F10 | Planned (P3) |
| Shelf | Drag anything onto the shelf; drag it out on another computer | F4 | Planned (P3) |
| Compute | Run a supported job on the best device; watch it; cancel | F8 | Planned (P3) |
| Desk | One keyboard and mouse across both screens | F2 | Planned (P4, needs both machines side by side) |
| Network | Share `localhost:3000` with your other devices | F3 | Planned (P4) |
| Drive | Open shared folders from another computer on demand | F12 | Planned (P4) |
| Rules | Build "when this happens, do that" automations | F13 | Later (P6) |
| Timeline | Scroll back through what happened and restore it | F14 | Later (P6) |
| Handoff | Resume apps at the same state on another computer | F7 | Later (P6) |
| Search | One box over everything on every device | F5 | Later (P6) |
| Assistant | Ask an AI about your devices | F6 | Last (P7) |

## Phases

Each phase ends with something usable and something measurable.

| Phase | Ships | Done when |
| --- | --- | --- |
| **P0 · Security foundation** ✅ | Device identity keys in the OS keychain; pairing with a short code (SPAKE2 bound to the TLS channel); mutual TLS with pinned keys; signed operations; membership and revocation in the log; limits on every decoder and peer; property-based fuzzing of the decoder; `cargo deny` and `npm audit` in CI; content policy (secrets, pastejacking); durable log re-verified on load | **Done 2026-11-23.** V1 to V5 closed, each covered by unit, end-to-end or simulator tests (SECURITY.md). Deferred: capability grants land with the first feature that needs one (F8 tasks, P3); `cargo-fuzz` targets (nightly) alongside the stable property tests. |
| **P1 · Real app** (all built; the two-machine check waits for a second device) | Tauri shell and tray, open at sign-in; authenticated local channel between UI and daemon; the UI's sample data replaced with live data; OS clipboard change events instead of polling; the log saved to SQLite; reconnect with backoff; the `tether` CLI | Copy on one computer, see it in the Tether window on the other, paired and encrypted, after a reboot of either |
| **P2 · Core correctness and quick wins** (quick wins done 2026-11-23) | Merge rules for pins, deletes and device names; notifications, send to one device, quick paste palette; still open: compaction, content detection in Rust, simulator faults for crashes, clock skew and revocation mid-sync | 10,000 seeds pass with the new invariants; p50/p99 clipboard latency between the owner's two Windows machines published in the README |
| **P3 · Beyond the clipboard** (agreed order 2026-11-23) | F11 Notes; F10 Pulse; F4 Drop shelf; F8 Compute | Concurrent offline edits converge with nothing lost or interleaved (simulator); a 10 GB shelf item resumes after a cable pull with no chunk re-sent; a job runs on the desktop from the laptop, streams progress and cancels cleanly |
| **P4 · One desk** | F2 Flow; F3 localhost bridge; F12 Drive (read-only first) | Measured input latency and handover between the two machines, also during a large transfer |
| **P5 · Beyond the home network** | Signaling Worker, hole punching, encrypted relay server; F15 device relay | Clipboard, notes and the shelf work between two different networks |
| **P6 · Automation and continuity** | F13 Rules; F14 Timeline; F7 Handoff; F5 Search; F9 Settings; Vault | An event seen by two devices runs its rule once; a workspace resumes in one click |
| **P7 · Intelligence** | F6 AI over MCP | An agent answers questions about your devices with no data leaving them; every action confirmed and logged |
| **P8 · Mobile and sharing** | Android, then iOS; Spaces shared with other people | A phone joins with the same pairing flow |

## Decisions to make along the way

- **Sign-in with an account (decided 2026-11-23):** not now. Possibly later, and only to help a new device *find* yours; an existing device must still approve it. Trust always comes from pairing and device keys, never from the account, because an account that can add devices would let whoever controls it (or the account provider) read your data.
- **Input sharing** ships Windows ↔ Windows first (the owner's desktop and laptop). macOS needs the Accessibility permission, and Wayland needs a portal; both follow, and stay marked unverified until run on real machines.
- **AI integration** follows the MCP specification current at the time; capability names above are Tether's own.
- **Real-time co-editing (decided 2026-11-23):** DESIGN.md listed it as a v1 non-goal. Notes (F11) deliberately adds it for Tether's own notes only; Tether still does not sync keystrokes inside other apps.
