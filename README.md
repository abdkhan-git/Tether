# Tether

Peer-to-peer, end-to-end encrypted sync for your clipboard, files and workspaces across Windows, macOS and Linux. No server holds your data.

The core is a **replicated operation log**: every change is an immutable operation in its author's log, and devices sync by exchanging version vectors and sending each other what is missing. Correctness is checked by a **deterministic simulator** that runs thousands of seeded fault scenarios.

> Status: stage 1 in progress. Clipboard text syncs between devices on one network. Pairing, file transfer and the tray app are next.

## Layout

| Path | What it is |
| --- | --- |
| `crates/tether-core` | No-I/O core: device ids, hybrid logical clock, operations, the op log, wire messages |
| `crates/tether-daemon` | `tetherd`: mDNS discovery, QUIC transport, clipboard watcher |
| `crates/tether-sim` | Deterministic simulator: N devices, dropped/cut/reordered messages, partitions, clock skew |
| `apps/desktop` | Tauri tray app (not started) |

## Quick start

Install Rust from <https://rustup.rs>, then:

```sh
cargo test --workspace                       # unit tests + 1,000 simulated seeds
cargo run --release -p tether-sim -- --seeds 10000
```

On Windows, if linking fails with `LNK1104` the repo path is too long for the linker (260-character limit). Set `CARGO_TARGET_DIR` to a short folder such as `C:\t\tether`.

**Two daemons on one machine** (they would share one clipboard, so use stdin/stdout):

```sh
cargo run -p tether-daemon -- --stdin --name left
cargo run -p tether-daemon -- --stdin --name right
```

Type a line in one terminal; it appears in the other, and the sender logs the round-trip time.

**Two machines on one network:** run `cargo run --release -p tether-daemon` on both. Copy text on one and paste on the other. If your network blocks multicast, pass `--peer <ip>:<port>` (the port is printed at startup, or fix it with `--port`). Windows will ask to allow network access the first time.

## How a copy travels

1. The clipboard watcher sees new text.
2. The daemon appends `Op { device, seq, hlc_ts, ClipboardText }` to its log.
3. It sends the op to every connected peer on a new QUIC stream.
4. The peer applies it (duplicates are ignored, gaps trigger a pull), and if it is now the newest item, writes it to the clipboard.
5. On reconnect, each side sends its version vector and receives everything it missed.

Clipboard history is ordered by hybrid logical clock time with the device id as a tiebreak, so every device shows the same order even with skewed clocks.

## Stage 1 checklist

- [x] Cargo workspace, CI on three OSes plus 10k simulated seeds
- [x] Hybrid logical clock, op log, version-vector catch-up
- [x] mDNS discovery and QUIC connections (one side dials, by device id)
- [x] Clipboard text sync with round-trip latency logging
- [ ] Replace clipboard polling with OS change notifications (removes up to 100 ms)
- [ ] Pairing: Ed25519 device keys, Noise handshake with a short code, pinned keys (today any peer on the LAN can connect)
- [ ] Reconnect with backoff
- [ ] File send (single stream, then chunked and resumable in stage 3)
- [ ] Tauri tray app: devices, clipboard history, send file
- [ ] Benchmarks: copy-to-paste latency p50/p99 on LAN

## Benchmarks

| Metric | Target | Measured |
| --- | --- | --- |
| Clipboard copy to peer (LAN), p50 | < 50 ms | |
| Clipboard copy to peer (LAN), p99 | < 150 ms | |
| Simulated seeds per CI run, violations | 10,000, 0 | |
