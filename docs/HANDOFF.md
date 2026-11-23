# Handoff: where Tether stands and what comes next

This file lets a new session pick up the work without guessing. When the owner says **"let's continue"**, the next session should:

1. Read this file first, then the docs it links for the task at hand.
2. Check that the repository still matches this file: run `git log --oneline -5` and `git status`, and open the files named under "Next up". **Trust the code over this file.** If they disagree, say so and fix this file.
3. Start on the first unchecked item under "Next up". Say in one or two lines what you are about to do, then do it.
4. When a task or a session ends, update this file. Tick finished items, rewrite "Current state" and "Next up", and add a line to the session log. Then give the owner the git commands to run.

Keep this file short and factual. Every claim here should be something the code or a test run can confirm.

---

## Working agreements (always apply)

- **The owner runs git.** Never commit or push. Give the commands in a PowerShell block. Use no `Co-Authored-By` line or any other Claude attribution, in commit messages or anywhere else.
- **Security is a hard requirement.** Every change follows the rules in [SECURITY.md](SECURITY.md). A new feature needs a threat model (template at the end of SECURITY.md) and a test or simulator invariant for its security property.
- **The simulator stays green** after any change to sync logic. A new feature adds an invariant or a fault.
- **Windows first.** The owner works on Windows 11. Use PowerShell commands, Ctrl shortcuts, and File Explorer wording.
- **UI taste:** a neutral, Apple-like look built from library components. See [FRONTEND.md](FRONTEND.md).
- **Report honestly.** Say what was measured and what was not. Name anything untested.

---

## Current state (2026-11-23)

**The last finished phase is P0, the security foundation.** It is committed as `674113e`.

Verified on Windows:

- **Rust tests:** `cargo test --workspace` passes all 45: 34 core, 5 daemon unit, 4 end-to-end and 2 simulator.
- **Lints:** `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` are clean.
- **Simulator:** `cargo run --release -p tether-sim -- --seeds 10000` gives 0 failures across 2.0M ops, 201k network faults, 149k injected forgeries (none accepted) and 3,923 revocations. It took 206 s on 12 threads.
- **Supply chain:** `cargo deny check advisories licenses bans sources` passes. The duplicate-version warnings are expected (spake2 pulls in curve25519 v4). `npm audit` reports 0 vulnerabilities.
- **Frontend:** `cd app && npm run build` passes, including 36 content-detection tests.

Not verified yet:

- Sync between two physical machines.
- The real OS clipboard on two operating systems.
- Saving a key in the Windows Credential Manager. The tests use `--ephemeral` keys.

### Where things live

| Area | Files |
| --- | --- |
| Identity and signatures | `crates/tether-core/src/identity.rs` (`DeviceId` is the Ed25519 public key; `DeviceKey`; `Signature`) |
| Operations | `crates/tether-core/src/op.rs` (`Op`, `OpKind::{ClipboardText, DeviceAdded, DeviceRevoked}`, signed with domain `tether/op/v1`) |
| Log, membership, revocation | `crates/tether-core/src/log.rs` (`OpLog`, `apply_remote` runs cheap checks before verifying the signature, `apply_batch`, `derive_trust`, `missing_for` paging) |
| Wire protocol and limits | `crates/tether-core/src/wire.rs`, `crates/tether-core/src/limits.rs` (strict decode, property-fuzzed) |
| Pairing (SPAKE2 bound to TLS) | `crates/tether-core/src/pairing.rs`, `crates/tether-daemon/src/pair.rs` |
| Content policy (secrets, pastejacking) | `crates/tether-core/src/content.rs` |
| Clocks | `crates/tether-core/src/hlc.rs`, `crates/tether-core/src/clock.rs` |
| Mutual TLS with pinned keys | `crates/tether-daemon/src/tls.rs` (`Gate`, client and server verifiers) |
| Daemon state and policy | `crates/tether-daemon/src/ctx.rs` (never hold its mutex across `.await`) |
| Networking | `crates/tether-daemon/src/net.rs` (accept loop, sessions, pull and broadcast, reconnect every 5 s) |
| Storage | `crates/tether-daemon/src/store.rs` (append-only file, re-verified on load; **plain text at rest**), `keystore.rs` (keyring) |
| Commands | `crates/tether-daemon/src/control.rs` (`/pair`, `/join`, `/peers`, `/revoke`, `/whoami`, `/help`), `input.rs` (clipboard polled every 100 ms) |
| End-to-end security tests | `crates/tether-daemon/tests/security_e2e.rs` |
| Simulator | `crates/tether-sim/src/lib.rs` (attacks, revocation, invariants), `main.rs` (parallel seeds) |
| UI | `app/src/modules/*` (sample data only, not connected to the daemon), `app/src/lib/content/*` |
| CI and supply chain | `.github/workflows/ci.yml`, `deny.toml` |

---

## Next up: P1, the real app

**Goal** (from [ROADMAP.md](ROADMAP.md)): copy on the PC, see it in the Tether window on the MacBook. The two devices are paired and the connection is encrypted, and this still works after either one reboots.

The steps below are in order, because each depends on the ones before it.

- [ ] **1. Encrypted storage.** Replace `store.rs` with SQLite, encrypted at rest.
  - The database key is kept in the credential store next to the device key.
  - Keep the guarantees the current store has: everything is re-verified on load, and a crash cannot corrupt the database.
  - Migrate or discard the old append-only file.
  - *Decision needed:* `deny.toml` bans openssl, which rules out SQLCipher built against OpenSSL. **Recommended:** `rusqlite` (bundled) with each row sealed by XChaCha20-Poly1305 (`chacha20poly1305` crate). The row's id goes in as associated data, so a row cannot be swapped for another.
  - Tests: a tampered row is rejected; a torn write is survived; the key never appears in logs.
- [ ] **2. Local control channel.** The UI and CLI talk to the daemon over a channel only the current user can open.
  - Windows: a named pipe with an owner-only ACL. Unix: a `0600` socket.
  - Every session authenticates with a per-session token.
  - Messages are length-prefixed and bounded (SECURITY.md rules 6 and 9).
  - The API covers status, devices, clipboard history (paged), `pair`, `join`, `revoke`, and an event stream.
  - Write the threat model first. Tests: another user cannot connect; a wrong or missing token gets nothing.
- [ ] **3. `tether` CLI** (new binary). Commands: `tether status`, `tether devices`, `tether pair`, `tether join CODE [ADDR]`, `tether revoke ID`, `tether history`. It goes through the control channel and never reads the store directly.
- [ ] **4. OS clipboard change events** instead of polling every 100 ms.
  - Windows: `AddClipboardFormatListener`.
  - macOS: watch `changeCount` (there is no change notification).
  - Linux: X11 XFixes, with polling as the fallback.
  - This needs `unsafe`, so it goes in a new OS-integration crate (for example `tether-os`). Every `unsafe` block gets a `// SAFETY:` comment, and every other crate keeps `#![forbid(unsafe_code)]`.
- [ ] **5. Reconnect with backoff.** Replace the fixed 5 s loop in `net.rs` with exponential backoff and jitter. Reset it when a connection succeeds. Make it testable without sleeping.
- [ ] **6. Tauri shell and tray.**
  - Wrap `app/` in Tauri v2.
  - Strict CSP, no remote content, and only allow-listed commands (SECURITY.md rule 10).
  - The Tauri side is a thin client of the control channel; the daemon stays the only privileged process.
  - Replace the sample data in `app/src/modules/*` with live data (devices, clipboard, pairing flow).
  - Tray menu: open, pause sync, quit.
- [ ] **7. Two-machine check.** Pair two real machines. Copy text and check it arrives. Reboot each machine and check sync resumes. Record what was measured.
- [ ] **8. Close out P1.**
  - Update ROADMAP.md (mark P1 done), SECURITY.md (status table; the residual risk "history is plain text" is closed), PROJECT_CONTEXT.md (build status), README and this file.
  - Give the owner the git commands.

**After P1:** P2, core correctness. That covers merge rules for pins and deletes, compaction, content detection moved into Rust, simulator faults for crashes and clock skew, and published p50/p99 latency measured on two real machines.

---

## Open decisions

| Decision | Options | Recommendation |
| --- | --- | --- |
| Encryption at rest (P1.1) | SQLCipher (needs OpenSSL, which is banned) · encrypt each row in the app · a pure-Rust database | Encrypt each row in the app on `rusqlite` bundled |
| OS-integration crate name | `tether-os` · `tether-platform` | `tether-os` |
| How the UI reaches the daemon | Tauri spawns `tetherd` as a sidecar · `tetherd` runs as a per-user startup service | Sidecar for P1, a service later |

---

## Gotchas

- Run the simulator with `--release`. In debug builds it is far slower because every op is signed and verified.
- The end-to-end tests start real `tetherd` processes with `--ephemeral --stdin --no-mdns --port 0`. They never touch the system clipboard or the keychain.
- `deny.toml` bans openssl and wildcard versions. An internal path dependency needs `version = "0.1.0"`.
- The frontend pins `vitest@^3.2` because the latest Vitest did not work with Vite 6.
- On Windows, edit files with the Edit tool rather than heredocs in the shell, because quoting breaks.

---

## Session log

| Date | Session | Outcome |
| --- | --- | --- |
| 2026-11-23 | Initial build | Rust core, daemon, simulator, UI prototype |
| 2026-11-23 | Frontend | Neutral Apple-like UI, content-aware clipboard; approved by the owner |
| 2026-11-23 | P0 security foundation | V1 to V5 closed; 45 tests; 10k simulator seeds green; committed `674113e` |
| 2026-11-23 | Handoff | Added this file; P1 broken into steps |
