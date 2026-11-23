# Tether

Peer-to-peer, end-to-end encrypted sync for your clipboard, files and workspaces across Windows, macOS and Linux. No server holds your data.

The core is a **replicated operation log**: every change is an immutable, **signed** operation in its author's log, and devices sync by exchanging version vectors and sending each other what is missing. Who belongs to the group is recorded in the same log. Correctness and security are checked by a **deterministic simulator** that runs thousands of seeded scenarios with network faults, clock skew, forged operations and compromised devices.

> Status: phase P0 (security foundation) done. Devices pair with a six-digit code and only paired devices can connect. The desktop app's UI exists in `app/` with preview data; wiring it to the daemon is phase P1. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Security at a glance

Full model in [docs/SECURITY.md](docs/SECURITY.md).

- **Identity is a key.** Each device has an Ed25519 key in the OS credential store; its public key is its id.
- **Pairing with a code.** SPAKE2 on a six-digit code, bound to the TLS channel: one guess per attempt, three attempts per two-minute window, nothing to brute-force offline.
- **Only paired devices connect.** Mutual TLS 1.3 with pinned keys; a stranger's handshake fails before any data is exchanged.
- **Every operation is signed** and verified before it is stored; membership and revocation are signed operations too, enforced by every device.
- **Secrets stay local.** Copies that look like keys, tokens or passwords are never synced.
- **No pastejacking.** Text from a peer that looks like a shell command, or hides control characters, is kept in history instead of being put on your clipboard.
- **Everything is bounded:** message sizes, operations per message, streams per second, connections, timeouts.

## Layout

| Path | What it is |
| --- | --- |
| `crates/tether-core` | No-I/O core: device keys, signed operations, the op log with membership, pairing (SPAKE2), wire protocol with limits, content policy |
| `crates/tether-daemon` | `tetherd`: keychain identity, pinned mutual-TLS QUIC, pairing, mDNS discovery, clipboard watcher, durable log |
| `crates/tether-sim` | Deterministic simulator: faults, partitions, clock skew, forgeries, compromised and revoked devices |
| `app` | Desktop UI (React + TypeScript), see [docs/FRONTEND.md](docs/FRONTEND.md) |

## Quick start

Install Rust from <https://rustup.rs>, then:

```sh
cargo test --workspace                               # unit, simulator and end-to-end security tests
cargo run --release -p tether-sim -- --seeds 10000   # 10,000 adversarial scenarios, in parallel
```

On Windows, if linking fails with `LNK1104` the repo path is too long for the linker (260-character limit). Set `CARGO_TARGET_DIR` to a short folder such as `C:\t\tether`.

**Two daemons on one machine** (they would share one clipboard, so use stdin/stdout, and give each its own profile):

```sh
cargo run -p tether-daemon -- --stdin --profile left  --port 5001
cargo run -p tether-daemon -- --stdin --profile right --port 5002
```

In the first, type `/pair`; it prints a code. In the second, type `/join <code> 127.0.0.1:5001`. Then type a line in one terminal and it appears in the other. `/peers` lists the group and `/revoke <id>` removes a device. Each profile's key lives in the OS credential store; add `--ephemeral` for a throwaway identity that stores nothing.

**Two machines on one network:** run `cargo run --release -p tether-daemon` on both, type `/pair` on one and `/join <code>` on the other (devices are found over mDNS; if your network blocks multicast, add the address: `/join <code> <ip>:<port>`). Then copy on one and paste on the other. Windows asks to allow network access the first time.

## How a copy travels

1. The clipboard watcher sees new text. If it looks like a secret, it stops here.
2. The daemon appends a signed `Op { device, seq, hlc_ts, ClipboardText }` to its log and saves it.
3. It sends the op to every connected peer, each over a mutually authenticated QUIC connection.
4. The peer checks the limits, the author's membership and the signature, then applies it (duplicates are ignored, gaps trigger a pull). If it is now the newest item and passes the content policy, it goes on the clipboard.
5. On reconnect, each side sends its version vector and receives everything it missed, page by page.

Clipboard history is ordered by hybrid logical clock time with the device id as a tiebreak, so every device shows the same order even with skewed clocks.

## Progress

- [x] Cargo workspace, CI on three OSes, 10k simulated seeds, supply-chain checks
- [x] Hybrid logical clock, op log, version-vector catch-up
- [x] mDNS discovery and QUIC connections (one side dials, by device id)
- [x] Device keys in the OS credential store; pairing with a code; pinned mutual TLS
- [x] Signed operations; membership and revocation in the log
- [x] Limits, timeouts and rate limits; content policy (secrets, pastejacking)
- [x] Durable log on disk, re-verified on load; reconnect loop
- [ ] Desktop app wired to the daemon over an authenticated local channel (P1)
- [ ] Clipboard change notifications instead of polling (removes up to 100 ms)
- [ ] SQLite storage with encryption at rest (P1)
- [ ] Benchmarks: copy-to-paste latency p50/p99 between two real machines

## Benchmarks

| Metric | Target | Measured |
| --- | --- | --- |
| Clipboard copy to peer (LAN), p50 | < 50 ms | not yet measured between two machines |
| Clipboard copy to peer (LAN), p99 | < 150 ms | not yet measured between two machines |
| Simulated seeds per CI run, violations | 10,000, 0 | 10,000 seeds, 0 violations: 2.0M ops, 201k network faults, 149k attacks, 3,923 revocations (Windows 11, 12 threads, release build, 206 s) |
