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

**P0, the security foundation, is done** (`674113e`). **P1 is in progress:** steps 1 to 4 are committed (encrypted storage `9f6de96`, control channel `2cc9f81`, CLI `8c725bb`, clipboard events `5a2b601`); step 5, reconnect with backoff, is done (not yet committed when this was written).

Verified on Windows:

- **Rust tests:** `cargo test --workspace` passes all 106: 6 `tether-cli`, 43 core (9 for backoff), 20 daemon unit (9 store, 7 clipboard logic, 1 privacy mark), 4 CLI end-to-end, 6 control-channel end-to-end, 4 security end-to-end, 10 `tether-ipc`, 11 `tether-os` and 2 simulator. 4 more are `#[ignore]`d because they use the real clipboard or credential store; all 4 pass when run by hand (see below).
- **Reconnect (run by hand):** `cargo test -p tether-daemon --test reconnect_e2e -- --ignored --nocapture`. Two persistent daemons, A hard-killed: after a 6 s outage B reconnects 0.9 s after A is back (stateless reset); after a 25 s outage, 2.8 s (backoff: 932 ms, then 1,829 ms). History survives the kill. The test deletes its profiles, even when it fails. Without the derived connection-ID key the quick case takes 14 s and the test fails.
- **Real clipboard (run by hand):** `cargo test -p tether-os --release -- --ignored --test-threads=1 --nocapture` (marks detected; change detected in p50 1.1 ms, p99 7.6 ms over 100 copies) and `cargo test -p tether-daemon --release --test clipboard_e2e -- --ignored --nocapture` (copy on A to printed by B over loopback in p50 4.0 ms, p95 5.4 ms; all three privacy marks keep the copy on A; B's copy lands on A's clipboard and is not echoed). Windows 11, Intel Core i7-1265U, 15 GB. Both restore the clipboard text afterwards.
- **Mutation checks:** raising the client cap to 9, or making the server skip the client's proof, each makes a `control_e2e` test fail. Letting ESC through `term::safe` makes `cli_e2e::escape_sequences_from_a_peer_never_reach_the_terminal` fail. Making `marked_private` always false makes `clipboard_e2e` fail.
- **CLI, live:** the real `tether.exe` against a non-ephemeral daemon read the token from Credential Manager and ran `status`, `devices`, `history`, `pair --no-wait`, and a refused `revoke` of this device (exit code 1). With no daemon running it says "no control token for profile …: is tetherd running?". The test profile was deleted afterwards.
- **Control channel, live:** a non-ephemeral `tetherd` created `\\.\pipe\tether-<SID>-<profile>`; `Get-Acl` showed the owner as the user, a protected DACL, and one entry (the user, FullControl). The token was in Credential Manager. The test profile and its credentials were deleted afterwards.
- **Unix code:** `tether-os` (local channel, and clipboard: macOS `changeCount` and nspasteboard.org marks, X11 XFixes) passes clippy `-D warnings` for Linux and macOS, and `tether-ipc` for Linux, type-checked from Windows (`rustup target add x86_64-unknown-linux-gnu aarch64-apple-darwin`). None of it has run yet; CI runs the Unix tests, but CI has no desktop, so the macOS and X11 clipboard code needs a check on a real Mac and a Linux desktop. The macOS check of `tether-ipc` cannot run from Windows (blake3 needs a C compiler for that target).
- **Lints:** `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` are clean.
- **Simulator:** the last full run (at P0) gave 0 failures across 10,000 seeds: 2.0M ops, 201k network faults, 149k injected forgeries (none accepted), 3,923 revocations, 206 s on 12 threads. Re-run after P1.5 added `backoff` to `tether-core`: 10,000 seeds, 0 failures, the same 2.0M ops, 201k faults, 149k attacks and 3,923 revocations (416 s, slower than usual because the test suite ran at the same time). P1 has changed no sync logic in `tether-core`.
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
| Networking | `crates/tether-daemon/src/net.rs` (accept loop, sessions, pull and broadcast; `reconnect_loop` sleeps until a dial is due or a device event arrives), `tls.rs` (`transport()`: keep-alive 5 s, idle 15 s; `TlsIdentity::reset_key` and `cid_key`, derived from the device key) |
| Reconnect schedule | `crates/tether-core/src/backoff.rs` (`Policy`, `Redial`, `Outcome`; pure, with time and randomness passed in) |
| Storage | `crates/tether-daemon/src/store.rs` (SQLite `tether.db` in `%LOCALAPPDATA%\Tether\<profile>`; every value sealed with XChaCha20-Poly1305 and bound to its row; re-verified on load; imports and deletes the old P0 `ops.log`), `keystore.rs` (device key and database key in the credential store) |
| Commands | `crates/tether-daemon/src/api.rs` (what the daemon can be asked: status, devices, history paging, pair, join, revoke; shared by console and channel), `control.rs` (stdin console: `/pair`, `/join`, `/peers`, `/revoke`, `/whoami`, `/help`) |
| Clipboard | `crates/tether-os/src/clipboard.rs` (`watch`, `poll`, `sequence`, `marked_private`, `Mode`) with `clipboard/windows.rs` (message-only window plus `AddClipboardFormatListener`; privacy formats; `write_text_with_formats` for tests), `macos.rs` (`changeCount` every 50 ms; nspasteboard.org types), `x11.rs` (XFixes; polls under Wayland or with no display; no marks yet); `crates/tether-daemon/src/input.rs` (`Mirror`: one channel of `Clip::{Changed, Delivered}`, event bursts collapsed, own writes not echoed, snapshot read guarded by the sequence number) |
| Local control channel | `crates/tether-os/src/local.rs` + `local/windows.rs` (owner-only pipe, SID in the name, first-instance, the crate's only `unsafe`) and `local/unix.rs` (0600 socket in a checked 0700 dir, peer uid); `crates/tether-ipc` (`frame`, `auth` handshake, `proto`, `token` in the credential store, `client`); `crates/tether-daemon/src/ipc.rs` (server: 8 clients, 5 s auth, events via `Ctx::emit`) |
| CLI | `crates/tether-cli/src/lib.rs` (`Cli`, `Command`, `Target`, `run` writes to any `Write` and reads prompts from any `BufRead`), `term.rs` (`safe` sanitiser, previews, "3 min ago"), `main.rs` (the `tether` binary) |
| End-to-end tests | `crates/tether-daemon/tests/security_e2e.rs`, `control_e2e.rs`, `cli_e2e.rs` (CLI run in-process against real daemons), shared harness in `tests/common/mod.rs` (`Daemon::with_control` passes a token through `TETHER_CONTROL_TOKEN`) |
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
- [x] **3. `tether` CLI.** Done 2026-11-23 in `crates/tether-cli` (binary `tether`, global `--profile`). Threat model in SECURITY.md ("`tether` CLI").
  - `status`; `devices`; `history [-n N] [--full]` (pages through the daemon 100 at a time); `watch [--count N]` (stops after N copies); `pair [--no-wait]` (by default waits until a new member appears or the window closes); `join CODE [ADDR]`; `revoke ID [--yes]` (a prefix of at least 4 characters naming one current member; asks y/N; refuses this device and already-revoked ones).
  - Everything the daemon returns is printed through `term::safe`: C0/C1 controls and DEL become visible symbols, bidi controls become U+FFFD, and newlines are kept only with `--full`. Errors are sanitised too.
- [x] **4. OS clipboard change events.** Done 2026-11-23. Threat model in SECURITY.md ("Clipboard change events and app privacy marks").
  - Windows: `AddClipboardFormatListener`, no polling. macOS: `changeCount` every 50 ms. Linux: X11 XFixes; polling every 100 ms under Wayland or with no display. The daemon logs which mode it is in (`watching the clipboard mode=...`).
  - Added beyond the original plan: copies that their app marks private are kept local (Windows `ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, `CanIncludeInClipboardHistory=0`, `CanUploadToCloudClipboard=0`, and an unreadable flag counts as private; macOS `org.nspasteboard.ConcealedType` and `TransientType`). Content and mark are read as one snapshot, checked with the clipboard sequence number.
  - Deliveries from peers now apply at once (before, they waited for the next 100 ms poll).
  - Follow-ups: Linux marks (`x-kde-passwordManagerHint`); the Wayland data-control protocol; a check of the macOS and X11 code on real desktops.
- [x] **5. Reconnect with backoff.** Done 2026-11-23. Threat model in SECURITY.md ("Reconnect with backoff").
  - `tether_core::backoff::Redial`: new targets are due at once; failures wait uniform in [ceiling/2, ceiling], where the ceiling doubles from 1 s and is capped at 30 s; a session of at least 30 s resets the backoff (redialled at once), a shorter one counts as a failure; never two dials in flight per target. Tested without sleeping (unit and property tests).
  - The daemon loop wakes when a dial is due or on any `Event::DevicesChanged`, with a 30 s safety re-check. `--peer` addresses use the same schedule. The `dialing` set in `ctx` is gone.
  - Found while testing: a hard-killed peer went unnoticed for 30 s (the old idle timeout), and a restarted peer could not reset the stale connection, because quinn's reset key and connection-ID key were random per process. Both keys are now derived from the device key; the idle timeout is 15 s.
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
- The end-to-end tests start real `tetherd` processes with `--ephemeral --stdin --no-mdns --port 0`. They never touch the keychain, and only the `#[ignore]`d ones (`clipboard_e2e`, and the `tether-os` clipboard tests) touch the system clipboard. Those overwrite what you have copied while they run and restore only text, so an image on the clipboard is lost.
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
| 2026-11-23 | P1.3 `tether` CLI | New crate `tether-cli`; terminal-escape sanitiser; pair waits for the join; revoke confirms; 85 tests; real binary checked against a live daemon |
| 2026-11-23 | P1.5 reconnect with backoff | `tether_core::backoff` (pure, property-tested); event-driven reconnect loop; QUIC reset and connection-ID keys derived from the device key; idle timeout 15 s; restarted peer reconnected in 0.9 s |
| 2026-11-23 | P1.4 clipboard events | `tether-os::clipboard` (Windows listener, macOS changeCount, X11 XFixes, polling fallback); password-manager privacy marks honoured; 97 tests plus 3 real-clipboard tests run by hand; copy to peer 4.0 ms p50 over loopback |
