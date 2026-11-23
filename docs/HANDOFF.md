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

**P0, the security foundation, is done** (`674113e`). **P1 is in progress:** step 1, encrypted storage, is committed (`9f6de96`); step 2, the local control channel, is done (not yet committed when this was written).

Verified on Windows:

- **Rust tests:** `cargo test --workspace` passes all 75: 34 core, 12 daemon unit (9 for the store), 6 control-channel end-to-end, 4 security end-to-end, 10 `tether-ipc`, 7 `tether-os` and 2 simulator.
- **Mutation check:** raising the client cap to 9, or making the server skip the client's proof, each makes a `control_e2e` test fail.
- **Control channel, live:** a non-ephemeral `tetherd` created `\\.\pipe\tether-<SID>-<profile>`; `Get-Acl` showed the owner as the user, a protected DACL, and one entry (the user, FullControl). The token was in Credential Manager. The test profile and its credentials were deleted afterwards.
- **Unix code:** `tether-os` passes clippy `-D warnings` for Linux and macOS, and `tether-ipc` for Linux, type-checked from Windows (`rustup target add x86_64-unknown-linux-gnu aarch64-apple-darwin`). The Unix tests have not run yet; CI will run them. The macOS check of `tether-ipc` cannot run from Windows (blake3 needs a C compiler for that target).
- **Lints:** `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` are clean.
- **Simulator:** the last full run (at P0) gave 0 failures across 10,000 seeds: 2.0M ops, 201k network faults, 149k injected forgeries (none accepted), 3,923 revocations, 206 s on 12 threads. P1.1 did not touch `tether-core` or `tether-sim`, so it was not re-run; its 2 test-suite seeds pass.
- **Supply chain:** `cargo deny check advisories licenses bans sources` passes. The duplicate-version warnings are expected (spake2 pulls in curve25519 v4); P1.1 added no new duplicates. `npm audit` reports 0 vulnerabilities.
- **Frontend:** `cd app && npm run build` passes, including 36 content-detection tests.
- **Real credential store:** with a throwaway profile, `tetherd` saved both keys in Windows Credential Manager (`device-key/<profile>.Tether`, `db-key/<profile>.Tether`). It kept the same identity across a hard kill, reloaded its operation (the next copy got `seq=2`), and no plain text reached the data folder. The profile was deleted afterwards.

Not verified yet:

- Sync between two physical machines.
- The real OS clipboard on two operating systems.
- macOS Keychain and Linux Secret Service (the code is the same `keyring` path; only Windows was run).
- That a second OS user is refused by the pipe or socket. This rests on the DACL and file modes, which the tests read back; no test runs as another account.
- That the control token is removed on a clean shutdown (Ctrl+C). The checks above stopped the daemon with a hard kill.

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
| Storage | `crates/tether-daemon/src/store.rs` (SQLite `tether.db` in `%LOCALAPPDATA%\Tether\<profile>`; every value sealed with XChaCha20-Poly1305 and bound to its row; re-verified on load; imports and deletes the old P0 `ops.log`), `keystore.rs` (device key and database key in the credential store) |
| Commands | `crates/tether-daemon/src/api.rs` (what the daemon can be asked: status, devices, history paging, pair, join, revoke; shared by console and channel), `control.rs` (stdin console: `/pair`, `/join`, `/peers`, `/revoke`, `/whoami`, `/help`), `input.rs` (clipboard polled every 100 ms) |
| Local control channel | `crates/tether-os/src/local.rs` + `local/windows.rs` (owner-only pipe, SID in the name, first-instance, the crate's only `unsafe`) and `local/unix.rs` (0600 socket in a checked 0700 dir, peer uid); `crates/tether-ipc` (`frame`, `auth` handshake, `proto`, `token` in the credential store, `client`); `crates/tether-daemon/src/ipc.rs` (server: 8 clients, 5 s auth, events via `Ctx::emit`) |
| End-to-end tests | `crates/tether-daemon/tests/security_e2e.rs`, `control_e2e.rs`, shared harness in `tests/common/mod.rs` (`Daemon::with_control` passes a token through `TETHER_CONTROL_TOKEN`) |
| Simulator | `crates/tether-sim/src/lib.rs` (attacks, revocation, invariants), `main.rs` (parallel seeds) |
| UI | `app/src/modules/*` (sample data only, not connected to the daemon), `app/src/lib/content/*` |
| CI and supply chain | `.github/workflows/ci.yml`, `deny.toml` |

---

## Next up: P1, the real app

**Goal** (from [ROADMAP.md](ROADMAP.md)): copy on the PC, see it in the Tether window on the MacBook. The two devices are paired and the connection is encrypted, and this still works after either one reboots.

The steps below are in order, because each depends on the ones before it.

- [x] **1. Encrypted storage.** Done 2026-11-23: `rusqlite` (bundled) with each value sealed by XChaCha20-Poly1305; the slot (table and row id) is the associated data. The key is in the credential store; there is a key check (a wrong key moves the database aside); WAL with `synchronous=FULL`; an exclusive lock; the P0 file is imported, zeroed and deleted. Threat model in SECURITY.md, 9 tests in `store.rs`.
  - *Remove after P1 ships:* the legacy import (`import_legacy` and friends at the bottom of `store.rs`).
- [x] **2. Local control channel.** Done 2026-11-23. Threat model in SECURITY.md ("Local control channel").
  - Windows: a named pipe `\\.\pipe\tether-<SID>-<profile>` with a protected DACL for the user's SID only, `FILE_FLAG_FIRST_PIPE_INSTANCE`, and remote clients rejected. Clients connect with `SECURITY_IDENTIFICATION`. Unix: a `0600` socket in a `0700` directory whose owner is checked, plus a peer-uid check.
  - Mutual authentication: a fresh 256-bit token at each start, kept in the credential store as `control-token/<profile>` and removed on a clean shutdown. Each side proves it knows the token with keyed BLAKE3 over both nonces; the token is never sent. `--ephemeral` daemons take the token from `TETHER_CONTROL_TOKEN` and have no channel without it.
  - Limits: 8 clients, 5 s to authenticate, 16 KiB requests, 4 MiB replies, history pages of 100 items or about 2 MiB, 10 s write timeout, 256 queued events per subscriber (then `Lagged`).
  - API (`tether_ipc::proto`): `Status`, `Devices` (members and unpaired devices nearby), `History { before, limit }` with cursor paging, `Pair`, `Join`, `Revoke`, `Subscribe` (events: `Clipboard`, `HistoryChanged`, `KeptLocal`, `DevicesChanged`, `Lagged`).
- [ ] **3. `tether` CLI** (new binary, for example `crates/tether-cli`). Commands: `tether status`, `tether devices`, `tether pair`, `tether join CODE [ADDR]`, `tether revoke ID`, `tether history`, plus `tether watch` (events). Use `tether_ipc::Client::connect(profile)`; resolve an ID prefix through `Request::Devices`, since `Revoke` takes a full `DeviceId`. It never reads the store directly.
- [ ] **4. OS clipboard change events** instead of polling every 100 ms.
  - Windows: `AddClipboardFormatListener`.
  - macOS: watch `changeCount` (there is no change notification).
  - Linux: X11 XFixes, with polling as the fallback.
  - This needs `unsafe`, so it goes in `tether-os` (it exists now; add a `clipboard` module). Every `unsafe` block gets a `// SAFETY:` comment (clippy enforces it there), and every other crate keeps `#![forbid(unsafe_code)]`.
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
| How the UI reaches the daemon | Tauri spawns `tetherd` as a sidecar · `tetherd` runs as a per-user startup service | Sidecar for P1, a service later |

---

## Gotchas

- Run the simulator with `--release`. In debug builds it is far slower because every op is signed and verified.
- The end-to-end tests start real `tetherd` processes with `--ephemeral --stdin --no-mdns --port 0`. They never touch the system clipboard or the keychain.
- `deny.toml` bans openssl and wildcard versions. An internal path dependency needs `version = "0.1.0"`.
- The frontend pins `vitest@^3.2` because the latest Vitest did not work with Vite 6.
- On Windows, edit files with the Edit tool rather than heredocs in the shell, because quoting breaks.
- The store holds an exclusive SQLite lock for as long as `Store` lives. In tests, shadowing a variable (`let l = ...` twice) does not drop the first one, so the second open fails: `drop` it explicitly.
- The store tests take about 7 s on Windows because every commit is synced to disk (`synchronous=FULL`). That is expected.
- If a cross-target `cargo clippy --target ...` seems to hang, it is probably waiting for the build lock held by the editor's rust-analyzer. Run it again once the editor is idle.
- Inspect the live pipe from PowerShell: `[System.IO.Directory]::GetFiles("\\.\pipe\") | ? { $_ -like "*tether-*" }`, then `Get-Acl` on that path.
- `cmdkey` arguments get mangled by Git Bash. Run it from PowerShell, for example `cmdkey /delete:"db-key/<profile>.Tether"`.

---

## Session log

| Date | Session | Outcome |
| --- | --- | --- |
| 2026-11-23 | Initial build | Rust core, daemon, simulator, UI prototype |
| 2026-11-23 | Frontend | Neutral Apple-like UI, content-aware clipboard; approved by the owner |
| 2026-11-23 | P0 security foundation | V1 to V5 closed; 45 tests; 10k simulator seeds green; committed `674113e` |
| 2026-11-23 | Handoff | Added this file; P1 broken into steps |
| 2026-11-23 | P1.1 encrypted storage | Confirmed P0 has nothing open beyond its planned deferrals; SQLite with per-row XChaCha20-Poly1305; 52 tests; checked with the real Windows Credential Manager |
| 2026-11-23 | P1.2 local control channel | New crates `tether-os` (owner-only pipe/socket) and `tether-ipc` (mutual token handshake, protocol, client); daemon `api.rs` shared by console and channel; 75 tests; live pipe ACL checked with `Get-Acl` |
