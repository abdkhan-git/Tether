# Tether: project context and decision history

A condensed record of the planning conversation (2026-11-23) that led to Tether. It captures what the owner wants, the ideas considered, why Tether won, and where the build stands. Read this together with [DESIGN.md](DESIGN.md).

## What the owner is optimizing for

- **Purpose of the project:** land a big-tech (FAANG-level) software engineering offer in the 2027 market. It must be resume-worthy and give deep material to discuss in interviews, not a cookie-cutter project.
- **Skills to showcase:** the "gritty" engineering that is valued more in the age of AI. An Amazon applied scientist told the owner that engineers who prioritize **latency, race conditions, concurrency and correctness** stand out. Distributed systems, networking and performance over frontend, CRUD or finance/stock apps.
- **Must solve a real problem** that the owner would use day to day as an individual, not a demo built only to exist.
- **Must be ambitious and non-obvious.** The owner rejected ideas that duplicate features mainstream products already have.
- **Cost:** as close to $0 as possible. Running locally is fine; hosted pieces must fit a free tier.
- **Language:** not limited to Go. Rust was chosen for the core.
- **Room to grow:** the owner wants to keep scaling features after the core works.

## Ideas considered and why they were dropped

| Idea | Verdict |
| --- | --- |
| LLM inference gateway (routing, semantic caching) | Only valuable at company scale; owner felt it read as a "token tracker" |
| Agent firewall for MCP / prompt injection | Timely, but its hard parts are security policy, not latency or concurrency |
| Local LLM inference engine (mini-vLLM) | Very gritty, but Ollama/llama.cpp already do the job, so no real daily purpose |
| Durable execution engine (Temporal-lite) | Strong fundamentals, but the user is a developer, not a person with a daily need |
| Real-time local voice assistant | Owner rejected it: mainstream LLM apps already have voice modes |
| Kafka-compatible log in Rust | Very gritty, but no daily use; Redpanda/WarpStream exist |
| Deterministic simulation tester (standalone) | Matches "race conditions" best, but abstract without a real system to test |
| Instant code-search daemon | Incremental over ripgrep/Zoekt |
| Distributed builds across own devices | Niche |
| "Why is my PC slow" kernel-tracing recorder | Steep, platform-specific start |
| Along-route stops for maps (owner's idea) | Useful but shallower; possible small side project later |
| Real-time transit reliability platform | Needs always-on ingestion; only useful to transit riders |

**On Kafka specifically:** do not bolt Kafka onto a personal project for resume value; interviewers will ask "why Kafka here?". Instead, Tether *builds* what Kafka is (a replicated, append-only log with offsets and compaction) because the problem needs it.

## Why Tether won

The cross-device workspace was the owner's own idea. It wins when combined with two of the other ideas:

1. **Daily purpose:** clipboard, files and workspace sync between the owner's own Windows/macOS/Linux machines.
2. **The Kafka story, earned:** each device keeps an append-only operation log replicated to peers (offsets = per-device `seq`, version vectors, compaction).
3. **Race conditions, proven:** a deterministic simulation harness (FoundationDB/TigerBeetle style) runs thousands of seeded fault scenarios and checks invariants. This turns "I built a sync app" into "I can prove my distributed system converges".
4. **Concrete latency numbers:** copy-to-paste latency, transfer throughput, resume time.

**Differentiation from LocalSend, KDE Connect, Syncthing and Apple Continuity:** works across all three desktop OSes, is correct while offline (backed by published simulation results), and offers workspace resume.

## Key decisions

- **Rust core** (tokio, quinn for QUIC, mdns-sd), **Tauri + React** for the desktop UI. Native app with tray, main window and CLI; **no web app** (a web page cannot read the clipboard in the background or open P2P sockets).
- **Peer-to-peer and end-to-end encrypted.** The only cloud piece is a Cloudflare Worker for signaling and a ciphertext-only relay fallback (free tier).
- **Hybrid logical clocks** for ordering across skewed clocks; clipboard history ordered by `(hlc_ts, device_id)` so every device shows the same order.
- **Pairing:** Ed25519 device keys, Noise handshake with a short code, pinned keys.
- **Files:** content-defined chunking (FastCDC), BLAKE3 hashes, resumable and deduplicated transfers.
- **Mobile is last** (iOS restricts background clipboard access).
- **Roadmap:** stages 1–4 (~16 weeks part-time) to a public v1 with published benchmarks. Stage 2 (replicated log + simulator) is the core and gets the most time.

## Later decisions (2026-11-23)

- **Frontend:** neutral, Apple-like UI in `app/` built from vendored React Bits and Magic UI components; content-aware clipboard previews. The owner rejected a generic sidebar look and a black-and-gold "luxury" look.
- **No account sign-in for trust.** Pairing and device keys decide trust; an account could at most help devices find each other later.
- **Security is a hard requirement.** docs/SECURITY.md holds the rules; P0 (security foundation) came before any new feature.
- **Feature direction:** "your computers act as one machine": flagships F1 to F10 in docs/ROADMAP.md.

## Build status (2026-11-23)

Done and verified on Windows:

- Phase P0 complete: Ed25519 device keys in the OS credential store; SPAKE2 pairing bound to the TLS channel; mutual TLS with pinned keys; signed operations; membership and revocation in the log; protocol limits and rate limits; content policy (secrets stay local, pastejacking held); durable log re-verified on load; reconnect loop.
- P1 step 1, encrypted storage: the log and group root live in SQLite (WAL, synchronous=FULL, exclusive lock), each value sealed with XChaCha20-Poly1305 under a key in the OS credential store and bound to its row. Checked on Windows with the real Credential Manager: the keys persist, the database survives a hard kill, and no plain text reaches disk.
- P1 step 2, local control channel: a named pipe with a protected, owner-only DACL (read back in tests and checked live with `Get-Acl`), or a 0600 Unix socket with peer-uid checks; both ends prove knowledge of a per-run token (keyed BLAKE3 over both nonces); bounded frames, 8 clients, 5 s to authenticate. API: status, devices, paged history, pair, join, revoke, events. The console uses the same code path.
- 75 Rust tests pass (core, daemon, ipc, os, simulator), including 6 end-to-end tests of the control channel and 4 end-to-end tests that run real daemons over loopback QUIC (stranger refused, pastejacking held, secrets kept local, brute force stopped, revocation enforced). Clippy (`-D warnings`) and rustfmt clean.
- Simulator: 10,000 seeds × 4 devices, 0 failures: 2.0M ops, 201k network faults, 149k forged or injected operations (none accepted), 3,923 compromised-and-revoked devices (206 s on 12 threads; seeds run in parallel because every op is now signed and verified, about 27 µs to sign and 84 µs to verify).
- Live sync between two paired daemons on loopback: about 0.8 ms round trip including signing and verification.
- Frontend: 36 unit tests for content detection; typecheck and build clean; `npm audit` clean.

Not done yet:

- Not tested between two physical machines or with the real clipboard on two OSes.
- Clipboard is polled every 100 ms; switch to OS change notifications (P1).
- The UI is not connected to the daemon (P1); no Tauri shell, no `tether` CLI, no file send.

## Environment notes

- Rust 1.100 stable and Visual Studio 2022 Build Tools (C++ workload) were installed on 2026-11-23.
- In the original temporary folder, linking failed with `LNK1104` because the path exceeded Windows' 260-character limit; the fix was a short `CARGO_TARGET_DIR`. In `IdeaProjects` this should not be needed.
