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

## Build status (2026-11-23)

Done and verified on Windows:

- Cargo workspace: `tether-core` (DeviceId, HLC, Op, OpLog with version vectors, wire `Message`), `tether-daemon` (`tetherd`: mDNS discovery, QUIC, clipboard polling, `--stdin` and `--peer` modes), `tether-sim` (deterministic simulator).
- 10 unit tests pass; clippy (`-D warnings`) and rustfmt clean.
- Simulator: 10,000 seeds × 4 devices converged with 0 failures (~2M ops, ~198k injected faults, 3.4 s).
- Two live daemons on one machine: catch-up after being offline works; live sync ~1.5 ms round trip on loopback; mDNS discovery works and only the lower device ID dials.
- GitHub Actions CI for Windows, macOS and Linux plus a 10k-seed simulation job (not yet run on GitHub).

Not done yet (stage 1 checklist in the README):

- Not tested between two physical machines or with the real clipboard.
- Pairing/authentication: today any peer on the LAN can connect (encrypted but unauthenticated, certificate verification skipped).
- Clipboard is polled every 100 ms; switch to OS change notifications.
- No reconnect with backoff, no file send, no `tether-cli`, no Tauri tray app.

## Environment notes

- Rust 1.100 stable and Visual Studio 2022 Build Tools (C++ workload) were installed on 2026-11-23.
- In the original temporary folder, linking failed with `LNK1104` because the path exceeded Windows' 260-character limit; the fix was a short `CARGO_TARGET_DIR`. In `IdeaProjects` this should not be needed.
