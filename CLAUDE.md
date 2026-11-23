# Tether

Peer-to-peer, end-to-end encrypted sync for clipboard, files and workspaces across Windows, macOS and Linux. Rust core on a replicated operation log, verified by a deterministic simulator.

## Continuing work

When the owner says "let's continue" (or starts a new session without a specific task), read [docs/HANDOFF.md](docs/HANDOFF.md) first. It holds the current state, the next unchecked step, open decisions and the working agreements. Check it against the code (`git log`, the files it names) before acting. At the end of every task or session, update HANDOFF.md: tick finished steps, rewrite "Current state" and "Next up", add a session-log line. The owner runs git: give commands, never commit, no Claude attribution.

## Reference docs

Before planning or changing anything, read:

- [docs/DESIGN.md](docs/DESIGN.md): the full design doc (architecture, sync protocol, security, roadmap).
- [docs/PROJECT_CONTEXT.md](docs/PROJECT_CONTEXT.md): why this project exists, what the owner is optimizing for (big-tech offer; latency, race conditions, correctness), ideas rejected, key decisions, and current build status.
- [docs/SECURITY.md](docs/SECURITY.md): threat model, the rules every change must follow, the capability model, and known open vulnerabilities. Security is a hard requirement from the owner.
- [docs/ROADMAP.md](docs/ROADMAP.md): the flagship features (F1 to F10), how each stays safe, and the phase order (P0 security foundation first).

## Layout

- `crates/tether-core`: no-I/O core (DeviceId, HLC, Op, OpLog, wire Message). Keep it free of I/O so the simulator can run it.
- `crates/tether-daemon`: `tetherd` binary (mDNS, QUIC via quinn, clipboard).
- `crates/tether-sim`: deterministic simulator and convergence invariants.
- `crates/tether-os`: OS integration, the only crate allowed `unsafe` (every block has a `// SAFETY:` comment, enforced by clippy). Holds the owner-only local channel (named pipe / Unix socket) and clipboard change detection plus app privacy marks (`clipboard`). Tests that touch the real clipboard are `#[ignore]`d; run them by hand with `-- --ignored`.
- `crates/tether-ipc`: the local control protocol (frames, mutual token handshake, requests/replies/events, client) used by the daemon, the CLI and the desktop app.
- `crates/tether-cli`: the `tether` command (status, devices, history, watch, pair, join, revoke). A client of the control channel only; everything it prints from the daemon goes through `term::safe` (no terminal escapes).
- `app/src-tauri`: the `tether-app` crate, the Tauri 2 shell (window, tray). A thin client of the control channel; the page may call only the eight commands in `build.rs` / `capabilities/main.json`, and the window refuses to navigate anywhere but the bundled page. Starts `tetherd --managed` from beside its own executable when no daemon answers.
- `app`: desktop UI (React + TypeScript + Tailwind v4 + Motion). Live data from the daemon inside the app, sample data in a plain browser. Features are modules in `app/src/modules/`, registered in `registry.ts`. Read [docs/FRONTEND.md](docs/FRONTEND.md) before touching it: it has the design language (neutral graphite, Apple-like; colour only as a status signal; no gold, purple or blue; no sidebar), the content-aware clipboard engine (`app/src/lib/content/`, unit-tested), platform awareness (owner is on Windows: Ctrl shortcuts, File Explorer wording), the vendored React Bits / Magic UI components, and how to add a module.

## Commands

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run --release -p tether-sim -- --seeds 10000
cargo run -p tether-daemon -- --stdin --profile left  # two daemons on one machine: /pair in one, /join CODE ADDR in the other
cargo run -p tether-cli -- --profile left status      # the CLI, talking to that daemon over the control channel
cd app && npm install && npm run app                 # the desktop app against the daemon (builds tetherd first)
cd app && npm run dev                                # browser preview at http://localhost:1420, sample data
cd app && npm run build                              # typecheck + tests + production build
cd app && npm run app:build                          # release app and installer, tetherd staged beside it
```

## Conventions

- Follow the rules in docs/SECURITY.md on every change: authenticate before serving anything, verify signed operations, never execute received content, gate powerful features behind capabilities that are off by default and confirmed on the affected device, bound every input, no `unsafe` outside the OS-integration crate, never log secrets. New features need a threat model (template in SECURITY.md) and a test or invariant for their security property.
- Every change to sync logic must keep the simulator green; add an invariant or fault when adding a feature.
- Never hold the `std::sync::Mutex` in the daemon across an `.await`.
- Performance claims in the README must be measured, with hardware and method stated.
