# Tether

Peer-to-peer, end-to-end encrypted sync for clipboard, files and workspaces across Windows, macOS and Linux. Rust core on a replicated operation log, verified by a deterministic simulator.

Before planning or changing anything, read:

- [docs/DESIGN.md](docs/DESIGN.md): the full design doc (architecture, sync protocol, security, roadmap).
- [docs/PROJECT_CONTEXT.md](docs/PROJECT_CONTEXT.md): why this project exists, what the owner is optimizing for (big-tech offer; latency, race conditions, correctness), ideas rejected, key decisions, and current build status.

## Layout

- `crates/tether-core`: no-I/O core (DeviceId, HLC, Op, OpLog, wire Message). Keep it free of I/O so the simulator can run it.
- `crates/tether-daemon`: `tetherd` binary (mDNS, QUIC via quinn, clipboard).
- `crates/tether-sim`: deterministic simulator and convergence invariants.
- `app`: desktop UI (React + TypeScript + Tailwind v4 + Motion, Tauri shell not yet added). Features are modules in `app/src/modules/`, registered in `registry.ts`. Read [docs/FRONTEND.md](docs/FRONTEND.md) before touching it: it has the design language (neutral graphite, Apple-like; colour only as a status signal; no gold, purple or blue; no sidebar), the content-aware clipboard engine (`app/src/lib/content/`, unit-tested), platform awareness (owner is on Windows: Ctrl shortcuts, File Explorer wording), the vendored React Bits / Magic UI components, and how to add a module.

## Commands

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run --release -p tether-sim -- --seeds 10000
cargo run -p tether-daemon -- --stdin --name left    # two daemons on one machine
cd app && npm install && npm run dev                 # UI at http://localhost:1420
cd app && npm run build                              # typecheck + tests + production build
```

## Conventions

- Every change to sync logic must keep the simulator green; add an invariant or fault when adding a feature.
- Never hold the `std::sync::Mutex` in the daemon across an `.await`.
- Performance claims in the README must be measured, with hardware and method stated.
