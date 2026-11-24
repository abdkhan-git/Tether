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

**P0, the security foundation, is done** (`674113e`). **P1:** steps 1 to 6 are committed (encrypted storage `9f6de96`, control channel `2cc9f81`, CLI `8c725bb`, clipboard events `5a2b601`, reconnect `21585a1`, Tauri shell and tray `6209553`). Steps 7 and 8 need the owner's laptop (see "Waiting for a second device"). Open at sign-in and **P2.1 pins and deletes** are committed (`7761b09`). **Since `7761b09`, not yet committed when this was written:** the four quick wins (rename devices, notifications, send to one device, quick paste palette), the device correction (Windows desktop + Windows laptop, no Mac) and the feature map in ROADMAP.md.

Verified on Windows:

- **Rust tests:** `cargo test --workspace` passes all 141: 18 `tether-app`, 6 `tether-cli`, 51 core (9 backoff, 5 pins and deletes, 1 rename, 2 send), 23 daemon unit, 4 CLI end-to-end, 12 control-channel end-to-end (rename, send and copy-here added), 4 security end-to-end, 10 `tether-ipc`, 11 `tether-os` and 2 simulator. 4 more are `#[ignore]`d because they use the real clipboard or credential store; all 4 pass when run by hand (see below). Frontend: 48 tests.
- **Simulator with the quick wins:** 10,000 seeds, 0 failures: 1.70M ops, 293,680 pins/unpins/deletes (98,091 deletes), 100,264 renames, 49,078 sends, 188,225 network faults, 150,183 attacks (forged renames and deletes included), 3,923 revocations; 183 s. `check_marks` now also compares every device's names with an oracle.
- **Quick wins in the release app, checked by hand** (DevTools protocol, two ephemeral `--stdin` daemons, 20 checks): renaming by clicking the name renames it everywhere; clicking "Send to Work laptop" puts the item on the laptop's clipboard; "Copy to this PC" goes through the daemon and adds no history item; the palette window's capability refuses delete, pin, send, rename, pair and open_link; pressing the real Alt+Shift+V shows the palette, typing filters, Enter copies the match here and hides it. Notifications were not seen on screen (unit-tested; Windows attributes toasts from `target\` builds to PowerShell).
- **Simulator with pins and deletes (P2.1):** `cargo run --release -p tether-sim -- --seeds 10000`: 0 failures; 1.70M ops, 294,579 pins/unpins/deletes (98,223 deletes), 199,718 network faults, 150,346 attacks (now including forged deletes of real items), 3,923 revocations; 200 s on 12 threads. `check_marks` compares every honest device with an oracle rebuilt from what the devices did. Mutation checks: making the oldest pin win fails 179 of 300 seeds; letting a pin outrank a delete fails 141 of 300.
- **Pins, deletes and open at sign-in in the app, checked by hand** (release build, DevTools protocol, ephemeral daemons): clicking "Pin everywhere" and "Delete everywhere" in the page reaches the daemon and updates the page; a deleted item cannot be pinned again (39 checks in all, with the P1.6 ones). The sign-in switch writes `"…\tether-app.exe" --hidden --profile X` (path quoted) to `HKCU\…\Run`, reads back, and removes it; a `--hidden` launch keeps the Tauri window invisible and still reaches the daemon; the plugin commands are refused (11 checks). Test profiles, credentials, logs and Run entries removed afterwards.
- **Desktop app (P1.6), checked by hand through the WebView2 DevTools protocol** against a release build (`npx tauri build --no-bundle`), with two ephemeral `--stdin` daemons so neither the clipboard nor the credential store was touched: the page loads with no CSP violations or errors; live status, devices (QUIC RTT 0.5 ms on loopback), pairing through the app's `pair` command, a copy from the peer, a held command, pause (nothing applied, still in history, banner shown), and revoke all work; five commands outside the allow-list are refused by Tauri's ACL; bad ids, codes, addresses and cursors are refused in Rust; `open_link` refuses file:, UNC, ms-msdt:, javascript: and credentialed links; navigating to a remote page and `window.open` are refused. The CSP is enforced, not just quiet: injected inline style and script, inline handlers, string eval from page code, remote fetch, images and iframes each raise a violation report. With a throwaway profile and no token, the app started its own `tetherd --managed`; killing the app made that daemon log "the app that started this daemon has gone", shut down, and remove its control token from Credential Manager. Profile, keys and logs deleted afterwards. `npx tauri dev -- -- --profile X` was checked the same way. The scripts were throwaway (scratchpad); there are no UI tests in CI.
- **Mutation checks (P1.6):** removing the paused check in `Ctx::local_copy` makes `control_e2e::a_paused_device_sends_nothing_and_applies_nothing_until_resumed` fail; removing it from `Ctx::apply` makes `ctx::tests::while_paused_…` fail.
- **Reconnect (run by hand):** `cargo test -p tether-daemon --test reconnect_e2e -- --ignored --nocapture`. Two persistent daemons, A hard-killed: after a 6 s outage B reconnects 0.9 s after A is back (stateless reset); after a 25 s outage, 2.8 s (backoff: 932 ms, then 1,829 ms). History survives the kill. The test deletes its profiles, even when it fails. Without the derived connection-ID key the quick case takes 14 s and the test fails.
- **Real clipboard (run by hand):** `cargo test -p tether-os --release -- --ignored --test-threads=1 --nocapture` (marks detected; change detected in p50 1.1 ms, p99 7.6 ms over 100 copies) and `cargo test -p tether-daemon --release --test clipboard_e2e -- --ignored --nocapture` (copy on A to printed by B over loopback in p50 4.0 ms, p95 5.4 ms; all three privacy marks keep the copy on A; B's copy lands on A's clipboard and is not echoed). Windows 11, Intel Core i7-1265U, 15 GB. Both restore the clipboard text afterwards.
- **Mutation checks:** raising the client cap to 9, or making the server skip the client's proof, each makes a `control_e2e` test fail. Letting ESC through `term::safe` makes `cli_e2e::escape_sequences_from_a_peer_never_reach_the_terminal` fail. Making `marked_private` always false makes `clipboard_e2e` fail.
- **CLI, live:** the real `tether.exe` against a non-ephemeral daemon read the token from Credential Manager and ran `status`, `devices`, `history`, `pair --no-wait`, and a refused `revoke` of this device (exit code 1). With no daemon running it says "no control token for profile …: is tetherd running?". The test profile was deleted afterwards.
- **Control channel, live:** a non-ephemeral `tetherd` created `\\.\pipe\tether-<SID>-<profile>`; `Get-Acl` showed the owner as the user, a protected DACL, and one entry (the user, FullControl). The token was in Credential Manager. The test profile and its credentials were deleted afterwards.
- **Unix code:** `tether-os` (local channel, and clipboard: macOS `changeCount` and nspasteboard.org marks, X11 XFixes) passes clippy `-D warnings` for Linux and macOS, and `tether-ipc` for Linux, type-checked from Windows (`rustup target add x86_64-unknown-linux-gnu aarch64-apple-darwin`). None of it has run yet; CI runs the Unix tests, but CI has no desktop, so the macOS and X11 clipboard code needs a check on a real Mac and a Linux desktop. The macOS check of `tether-ipc` cannot run from Windows (blake3 needs a C compiler for that target).
- **Lints:** `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check` are clean.
- **Simulator:** the last full run (at P0) gave 0 failures across 10,000 seeds: 2.0M ops, 201k network faults, 149k injected forgeries (none accepted), 3,923 revocations, 206 s on 12 threads. Re-run after P1.5 added `backoff` to `tether-core`: 10,000 seeds, 0 failures, the same 2.0M ops, 201k faults, 149k attacks and 3,923 revocations (416 s, slower than usual because the test suite ran at the same time). P1 has changed no sync logic in `tether-core`.
- **Supply chain:** `cargo deny check advisories licenses bans sources` passes with Tauri in the tree (more duplicate-version warnings, which are allowed: Tauri pulls in several `windows-sys` versions). `npm audit --omit=dev` (the CI gate) reports 0. Full `npm audit` now reports 3 advisories in dev-only `vitest` 3 and its `tinypool` (published after the last session; fixed in vitest 5, which needs a Vite upgrade). Follow-up below.
- **Frontend:** `cd app && npm run build` passes, including 44 tests (36 content detection, 8 for the daemon data mapping).
- **Real credential store:** with a throwaway profile, `tetherd` saved both keys in Windows Credential Manager (`device-key/<profile>.Tether`, `db-key/<profile>.Tether`). It kept the same identity across a hard kill, reloaded its operation (the next copy got `seq=2`), and no plain text reached the data folder. The profile was deleted afterwards.

Not verified yet:

- Sync between two physical machines.
- The real OS clipboard on two operating systems.
- macOS Keychain and Linux Secret Service (the code is the same `keyring` path; only Windows was run).
- That a second OS user is refused by the pipe or socket. This rests on the DACL and file modes, which the tests read back; no test runs as another account.
- The control token's removal after Ctrl+C in a terminal (it is now verified for the app's `--managed` shutdown, which runs the same code).
- The desktop app on macOS and Linux (tray, WKWebView, WebKitGTK). CI builds it on all three; it has only run on Windows.
- The tray menu's Quit item and the hide-on-close window were not clicked in the checks above (no way to drive the tray from DevTools). Quit runs the same shutdown as the crash path that was checked.

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
| Pause | `tether_ipc::Request::SetPaused`, `Status.paused`, `Event::Paused`; `crates/tether-daemon/src/ctx.rs` (`set_paused`, gates in `local_copy` and `apply`), `store.rs` (sealed `paused` flag in `meta`, unreadable means paused); CLI `tether pause` / `resume`; console `/pause` / `/resume` |
| Pins and deletes | `crates/tether-core/src/op.rs` (`OpKind::Pin`, `OpKind::Delete`, `ItemRef`, `item_hash`), `log.rs` (`Marks`, `OpLog::marks`, `item_ref`; `clipboard_history` skips deleted), `crates/tether-sim/src/lib.rs` (`check_marks` oracle, forged-delete attack); daemon `ctx.rs` (`Ctx::mark`, `Event::ItemChanged` in `apply`), `api.rs` (`set_pinned`, `delete`); app commands `set_pinned`, `delete_item` |
| Open at sign-in | `app/src-tauri/src/login.rs` (quoted login entry via `auto-launch`), commands `autostart` / `set_autostart` in `main.rs`, `--hidden` start |
| Quick wins | `OpKind::Rename` and `OpKind::Send` + `send_is_fresh` (`tether-core/src/op.rs`), `OpLog::renames`, `send_for_me`, `history_text` (`log.rs`); daemon `Ctx::rename`, `send`, `copy_here` and the send delivery in `apply`; app `notify.rs` (notifications, settings file), palette window and shortcut in `main.rs`, `capabilities/palette.json`; page `app/src/palette/` (`PaletteApp.tsx`, `search.ts`) |
| Desktop app (Tauri) | `app/src-tauri/src/main.rs` (the eight commands, window with navigation guard, setup), `link.rs` (connect or start `tetherd --managed`, request retry, event pump, shutdown, daemon log path), `guard.rs` (`is_app_url`, `external_link`), `view.rs` (JSON shapes and input checks), `tray.rs`; `build.rs` (command allow-list), `capabilities/main.json`, `tauri.conf.json` (CSP), `tauri.bundle.json` + `app/scripts/stage-tetherd.mjs` (installer) |
| UI | `app/src/modules/*`, `app/src/lib/content/*`; data: `app/src/data/store.ts` (`Backend`), `live.ts` (daemon), `daemon.ts` (typed commands and events), `map.ts` (+ tests), `mock.ts` (browser preview only) |
| CI and supply chain | `.github/workflows/ci.yml`, `deny.toml` |

---

## P1, the real app

**Goal** (from [ROADMAP.md](ROADMAP.md)): copy on one computer, see it in the Tether window on the other. The two devices are paired and the connection is encrypted, and this still works after either one reboots. The owner's machines are a Windows desktop and a Windows laptop.

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
- [x] **6. Tauri shell and tray.** Done 2026-11-23, committed `6209553`. Threat model in SECURITY.md ("Desktop app").
  - `app/src-tauri` (crate `tether-app`, Tauri 2.12, in the workspace). Thin client of the control channel; holds no keys, no history, no sockets.
  - The page's whole API is eight commands (`status`, `devices`, `history`, `pair`, `join`, `revoke`, `set_paused`, `open_link`), declared in `build.rs` so Tauri refuses anything `capabilities/main.json` does not grant. Every argument is checked in Rust.
  - CSP `default-src 'none'`, scripts and styles `'self'` only (the inline `<style>` in two vendored components moved to `index.css`), `connect-src ipc:` only, frozen prototype. The window refuses navigation away from the bundled page and new windows; links open only through `open_link` (http, https, mailto).
  - Daemon: the app uses a running daemon for its profile, or starts the `tetherd` next to its executable with the new `--managed` flag (no console; clean shutdown when stdin closes, so it never outlives the app). Log in `%LOCALAPPDATA%\Tether\logs\`; held items are now logged without their preview, so no clipboard text reaches that file.
  - Added beyond the original plan, because the tray needed it: **pause**, end to end (protocol v2 `SetPaused`, sealed and persisted, fail closed, `tether pause`/`resume`, console `/pause`/`/resume`). Also per-device QUIC RTT and op counts in `Devices`, shown in the UI.
  - Live data in Overview, Clipboard (paging, held items explained, "Held" filter), Devices (nearby list, real revoke, fingerprints), Diagnostics (real RTT, this device's version vector, clipboard ops), pairing (show a code and wait for the newcomer; enter a code with an optional address). Transfers and Workspaces are listed as coming inside the app. Rename, pin, delete and "keep local" are hidden in the app until the daemon supports them.
  - Tray: Open Tether, Pause sync (checked from the daemon's state), Quit Tether; tooltip shows connected devices. Closing the window hides it. Single instance for the default profile.
  - Follow-ups: "Show in File Explorer" for local paths only (refuse UNC); peers' OS and version vectors in the protocol; vitest 5 (dev-only advisories); UI tests in CI.
- [x] **6b. Open at sign-in.** Done 2026-11-23 (uncommitted). Threat model in SECURITY.md ("Open at sign-in").
  - A switch on the Overview's "This device" card, off by default; commands `autostart` / `set_autostart`. The entry runs `tether-app --hidden [--profile X]`, one per profile, and starts in the tray.
  - Uses `auto-launch` directly (`app/src-tauri/src/login.rs`), not `tauri-plugin-autostart`: both write the path unquoted, which on Windows is CWE-428 for paths with spaces. We quote it (Windows, Linux desktop entry; Linux refuses paths it cannot quote safely).

## Waiting for a second device

These need two real machines (the owner's Windows desktop and Windows laptop). Do them, in order, whenever the owner says the second device is available (CLAUDE.md, "Work that needs a second device"), then return to "Next up".

- [ ] **P1 step 7: two-machine check.** Pair two real machines. Copy text both ways and check it arrives. Turn on "Open at sign-in" on both, reboot each, and check sync resumes on its own. Record what was measured (time from sign-in to connected, copy-to-paste latency), with hardware, OS versions and network.
  - Build on each machine: `cd app; npm install; npm run app:build` (installer, `tetherd` staged beside the app) or `npm run app` (dev build). This is the first run between two physical machines (real LAN, mDNS across a router, Windows Firewall prompts on first start). The macOS and Linux desktop code still has no real run; say so in the record.
  - Also check across machines: pin on one, see it pinned on the other; delete on one, see it disappear on the other; pause on one.
  - If mDNS discovery fails between them, pair with the address shown in the pairing dialog (IP and port).
- [ ] **P1 step 8: close out P1.**
  - Update ROADMAP.md (mark P1 done), SECURITY.md (status table; the residual risk "history is plain text" is closed), PROJECT_CONTEXT.md (build status), README and this file.
  - Give the owner the git commands.
- [ ] **P2: published latency.** p50/p99 copy-to-paste latency between the two machines, measured over many copies, with method and hardware, for the README.

## Next up (one machine)

**Order agreed with the owner (2026-11-23):** first the quick wins, then the features that take Tether beyond the clipboard (F11 scratchpad, F4 files, F8 tasks), then the rest of P2's correctness work where a feature needs it (compaction before F4's large data, for example). AI (F6) is deliberately later. ROADMAP.md, "What you can do in the app", is the feature map. Each item gets a threat model, tests for its security property, and a simulator invariant if it touches sync.

Quick wins:

- [x] **QW1 Rename devices.** Done 2026-11-23 (uncommitted). `OpKind::Rename` (latest wins by HLC, device, seq; members only; names cleaned and bounded); `OpLog::renames()`; simulator oracle + forged renames (1,000 seeds, 9,982 renames, 0 failures); protocol v4 `Rename`; `tether rename ID NAME`; app `rename_device`; click a name on the Devices page to rename. Threat model "Device names".
- [x] **QW2 Notifications.** Done 2026-11-23 (uncommitted). `app/src-tauri/src/notify.rs`: "Copied on Laptop" with the length, never the text (OS notification centres store what they show on disk); none for own copies, while paused, while the window is focused, or when switched off; one per 3 s. Switch on the Overview's "This device" card; settings in `%APPDATA%Tetherpp-<profile>.json`. Uses `tauri-plugin-notification` from Rust only. Threat model "Notifications". Not yet seen on screen: in dev and target/ builds Windows attributes toasts to PowerShell; the installed app uses its own identity.
- [x] **QW3 Send to one device.** Done 2026-11-23 (uncommitted). A signed `OpKind::Send { item, to }` (chosen over a direct message: signatures, membership and once-only application for free). Only the target applies it, only within 2 min (`send_is_fresh`), under its content policy and pause; `OpLog::send_for_me`; protocol v5 `Send` / `Event::Received`; app `send_item`, "Send to <device>" buttons, toast and notification on the target. Threat model "Send to one device".
- [x] **QW4 Quick paste palette.** Done 2026-11-23 (uncommitted). Alt+Shift+V (not Ctrl+Shift+V, which other apps use for paste as plain text) opens an always-on-top `palette` window (`app/src/palette`); type to search the latest 100 items from every device, Enter puts the item on this clipboard and hides the palette, Esc or losing focus hides it. Its own capability (`capabilities/palette.json`): status, devices, history, copy_here only. New `Request::CopyHere` (protocol v6) puts an item on this device's clipboard through the daemon's clipboard thread, so it is not recorded again; "Copy to this PC" everywhere now uses it. If the shortcut is taken, the Overview says so. Threat model "Quick paste palette and copy here". Follow-up: paste into the previous app automatically (needs input injection in `tether-os`); a setting to change the shortcut.

Beyond the clipboard (P3):

- [ ] **F11 Live scratchpad:** design note first (CRDT choice: RGA vs Fugue; operation batching per keystroke burst; size limits), then `tether-core` CRDT with property tests, simulator invariant (convergence, no interleaving of concurrent runs), daemon + protocol, a Notes page.
- [ ] **F4 Files, stage 1:** send a file to a device; chunking, BLAKE3 per chunk, resume, quarantine folder; the Transfers page goes live.
- [ ] **F8 Run it on the big machine:** tasks defined on the target only; the `tasks.run` capability (the first capability grant); confirm on the target; streamed output; cancel.

Core correctness (P2), still open:

- [x] **P2.1 Pins and deletes.** Done 2026-11-23 (uncommitted). Threat model in SECURITY.md ("Pins and deletes").
  - `OpKind::Pin { item, pinned }` and `OpKind::Delete { item }` in `tether-core`; `ItemRef` names an item by `(device, seq)` and a BLAKE3 hash of its text, so a pin or delete cannot target a copy its author never saw. `OpLog::marks()` resolves them from the operations held: delete wins (tombstone); pins are last-writer-wins by (HLC time, device, seq). `clipboard_history()` leaves deleted items out.
  - Simulator: devices pin, unpin and delete; the adversary forges deletes; `check_marks` compares every honest device with an oracle.
  - Control protocol v3: `SetPinned`, `Delete`, `HistoryItem.pinned`, `Event::ItemChanged`. App commands `set_pinned`, `delete_item`. The Clipboard page's pin and "Delete everywhere" buttons work in the app; Pinned filter.
  - Follow-ups: `tether pin` / `tether delete` in the CLI (needs item ids in `tether history`); compaction to remove a deleted item's encrypted operation (P2.2); a confirm step or undo window before "Delete everywhere" (deletes are final).
- [ ] **P2.2 Compaction.** The log only grows, and a deleted item's encrypted operation stays in every database. Design how a device drops operations everyone has seen (a stable version vector across members) without breaking `missing_for` paging, re-verification on load, or a device that comes back after a long time offline. Redacting deleted text is the user-visible win. Needs a threat model and simulator invariants (convergence after compaction; a late device still catches up or is told to re-pair).
- [x] **P2.3 Settings merge: device names.** Done as QW1 above.
- [ ] **P2.4 Content detection in Rust.** Move the sensitivity rules the UI uses (`app/src/lib/content/detect.ts`) into `tether-core::content`, so the daemon, CLI and app agree on what is a secret.
- [ ] **P2.5 Simulator faults.** A device that crashes and restarts from its store (losing unsynced in-memory state), larger clock skew and jumps, and revocation in the middle of a sync page.

---

## Open decisions

| Decision | Options | Recommendation |
| --- | --- | --- |
| How the UI reaches the daemon | **Decided for P1 (2026-11-23):** a managed sidecar. The app uses a running daemon, or starts `tetherd --managed` beside it, which stops with the app. A per-user startup service remains the option for later. | Revisit with start at login |
| Start at login | **Decided (2026-11-23):** off by default, a switch on the Overview's "This device" card. | Done |
| Undo for "Delete everywhere" | Final, as now (delete wins) · a short undo window before the delete is sent · a "Recently deleted" view until compaction | An undo window before sending: it keeps delete-wins simple and protects against slips |
| Compaction (P2.2) | Drop operations every active member holds · keep everything, redact deleted text only · snapshots | Decide in P2.2's design; redacting deleted text is the first user-visible need |

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
- To inspect the desktop app, start it with `$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333"` and talk to `http://127.0.0.1:9333/json` with the Chrome DevTools Protocol (Node 22 has `WebSocket` built in). DevTools' own `Runtime.evaluate` is exempt from the CSP, so test the CSP by listening for `securitypolicyviolation` or running code from a page callback (string `setTimeout`), not by calling `eval` from DevTools.
- To point the app at an ephemeral daemon (no credential store), start `tetherd --ephemeral --stdin --profile X` and the app with `--profile X`, both with the same `TETHER_CONTROL_TOKEN`.
- A first `npx tauri build` takes about 8 minutes (Tauri in release); `cargo build -p tether-app` does not need `app/dist` (debug builds load from Vite), so CI's Rust jobs need no npm step.
- An inline `<style>` or `<script>` in the page is blocked in the app (it works in the browser preview). Vendored components refreshed from upstream must have theirs moved to `index.css` again.

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
| 2026-11-23 | P1.6 Tauri shell and tray | New crate `tether-app` (eight allow-listed commands, strict CSP, navigation guard, managed daemon); pause end to end (protocol v2, sealed, fail closed); live data in the UI; 118 Rust and 44 frontend tests; release build checked through WebView2 DevTools (34 + 15 checks) |
| 2026-11-23 | P1.6 follow-up and P2.1 | Open at sign-in (quoted login entry, fixing the unquoted path in tauri-plugin-autostart/auto-launch); pins and deletes on every device (delete wins, LWW pins, content-hash targets), 10,000 seeds green with an oracle; 129 Rust and 46 frontend tests; CLAUDE.md now lists the work waiting for a second device |
| 2026-11-23 | Quick wins | Corrected the device assumption (Windows desktop + Windows laptop); feature map in ROADMAP.md (F11 scratchpad added, AI later); rename devices, notifications (never the text), send to one device (fresh-only, signed), quick paste palette (Alt+Shift+V, own capability, copy-here via the daemon); 141 Rust and 48 frontend tests; 10,000 seeds with renames and sends |
