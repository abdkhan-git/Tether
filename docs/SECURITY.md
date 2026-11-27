# Tether security model

Tether runs on every computer a person owns, can type into them, and holds what they copy, including passwords. A flaw here harms the user directly, so security is a requirement on every change, not a feature to add later.

No one can promise software with zero vulnerabilities. This document makes a narrower promise that we can keep: a small attack surface, defence in depth, explicit rules that every change follows, and checks in CI that enforce them.

## Current status (2026-11-26)

Phase P0 is complete. The five issues found in the stage 1 prototype are fixed, each with a test that fails if the fix regresses:

| ID | Issue (stage 1) | Fix (P0) | Proven by |
| --- | --- | --- | --- |
| V1 | Any host on the LAN could connect and read the clipboard history. | Mutual TLS 1.3 with pinned Ed25519 device keys (`tether-daemon/src/tls.rs`). Unknown keys fail the handshake; while a pairing window is open they may only run the pairing ceremony. | `security_e2e::paired_devices_sync_and_strangers_get_nothing` |
| V2 | Received text went straight onto the system clipboard. | Content policy (`tether-core/src/content.rs`): commands that download-and-run, decode-and-run or destroy data, multi-line shell commands, and text with control or bidi characters are held in history. | `content::tests::pastejacking_shapes_are_held`, `security_e2e::incoming_commands_are_held_and_secrets_stay_local` |
| V3 | Identity was self-asserted; operations were unsigned. | Identity comes from the TLS certificate key; every operation is signed and verified; membership and revocation are signed operations (`tether-core/src/log.rs`). | `log::tests::strangers_and_forgeries_are_refused`, simulator (149k injected forgeries across 10,000 seeds, none accepted) |
| V4 | No limits. | Frame, operation, text, name and page limits checked on decode; 16 streams and 64 requests per second per connection; 32 connections; timeouts on handshakes, reads and pairing. | `wire::fuzz::*` (property tests on hostile bytes), `wire::tests::*` |
| V5 | Secrets were synced. | The daemon detects keys, tokens and passwords and never puts them in the replicated log. | `content::tests::known_secrets_are_detected`, `security_e2e::incoming_commands_are_held_and_secrets_stay_local` |

## Residual risks

Known limits of the current design, stated plainly:

| Risk | Why it remains | Plan |
| --- | --- | --- |
| A compromised member is fully trusted until revoked, and could race to revoke others. | All devices are equal; there is no higher authority to appeal to. | A recovery key or a quorum for revocations (later stage). |
| The encrypted database still shows how many items there are, roughly how long each is, and when it changed. | Rows are sealed one by one; padding every row would cost space for little gain. | Pad clipboard rows to size buckets if this ever matters. |
| An attacker who can write the database file can delete or roll back history (but not read, forge or reorder it). | Encryption gives confidentiality and per-row integrity, not freshness. | Peers re-send lost operations; a revoked device's operations are re-checked on load either way. |
| mDNS advertises that a machine runs Tether and its public device key. | Needed for zero-configuration discovery. | Option to turn discovery off (`--no-mdns` exists); rotating advertisement ids later. |
| The content policy is pattern-based and can miss a cleverly disguised command. | No heuristic catches everything. | It is defence in depth: only paired devices can send content at all. |
| Malware running as the same user can read the clipboard and the credential store. | Outside what an application can prevent. | Out of scope (see the adversary table). |

## What we protect, and from whom

**Assets:** clipboard contents and history; files; device identity keys; the ability to inject input or run actions on a device; the user's privacy (what they do, when, where).

| Adversary | In scope | Main defences |
| --- | --- | --- |
| Someone on the same network (café, campus, office) | Yes | Mutual authentication with pinned device keys; nothing is served before authentication. |
| The signaling / relay server, or whoever controls it | Yes | End-to-end encryption; the server sees device IDs and ciphertext only, and cannot add a device. |
| A paired device that is stolen or compromised | Yes | Revocation; per-device capabilities; dangerous actions need confirmation on the receiving device; rate limits; audit log. |
| Malicious content (crafted clipboard text, files, file names, oversized or deeply nested data) | Yes | Strict size and depth limits; no content is ever executed; paths sanitised; parsers fuzzed. |
| An AI agent driven by injected instructions (prompt injection) | Yes | Agents get read-only, scoped capabilities by default; actions need a human's confirmation on the device. |
| A malicious or hijacked dependency (supply chain) | Yes | Minimal dependencies, pinned lockfiles, `cargo-deny` / `cargo audit` / `npm audit` in CI, vendored UI code reviewed before use. |
| Malware already running as the same user on a device | Partly | It can already read that device's clipboard. We keep keys in the OS keychain and the local control channel accessible to the user only, and never make things worse. |
| An attacker with physical access to an unlocked device | No | Outside what an app can defend; revocation from another device limits the damage. |

## Rules every change follows

These are enforced in review and, where possible, by CI.

1. **Authenticate before anything else.** A connection that has not completed mutual authentication with a pinned, non-revoked device key receives nothing and is closed. No "public" endpoints.
2. **Every operation is signed** by its author's device key and verified before it is stored or applied. Unknown or revoked authors are rejected.
3. **Nothing received is ever executed.** Received text, files, links and file names are data. No feature passes network input to a shell, interpreter, `eval`, or the OS "open" handler without an explicit local user action.
4. **Least privilege by default.** Every powerful ability is a named capability (table below), off by default, granted per device, revocable, and shown in the UI.
5. **Confirm dangerous actions on the device they affect,** not the device that asks. A compromised laptop must not be able to act on the desktop by itself.
6. **Bound everything.** Every decoder has limits on message size, item count, string length and nesting depth. Every peer has rate limits and a cap on concurrent streams. Every queue is bounded.
7. **Memory-safe code.** `#![forbid(unsafe_code)]` in every crate except a small OS-integration crate, where each `unsafe` block carries a `// SAFETY:` comment and is reviewed.
8. **Secrets stay secret.** Keys live in the OS keychain (Windows Credential Manager, macOS Keychain, Secret Service), are zeroed in memory after use, and never appear in logs, crash reports, telemetry or the UI. Items detected as sensitive never leave the device that copied them.
9. **The daemon is the only privileged process.** The UI cannot touch the OS or the network directly; it asks the daemon over a local channel that only the current user can open (a named pipe with an owner-only ACL on Windows, a `0600` Unix socket elsewhere), authenticated with a per-session token.
10. **The UI loads nothing remote.** A strict Content Security Policy, no remote scripts or iframes, only allow-listed Tauri commands, and rendered content escaped (code highlighting HTML comes only from highlight.js, which escapes its input).
11. **Fail closed.** On any verification error, unknown message, or exceeded limit: drop the message, log it without its content, and keep the rest of the system running.
12. **Every feature ships with a threat model,** using the template at the end of this file, plus a simulator or test invariant for its security property.

## Capabilities

A device can do only what its peers have granted it. Grants are operations in the signed log, so every device agrees on them and they can be audited.

| Capability | What it allows | Default | Confirmation on the receiving device |
| --- | --- | --- | --- |
| `clipboard.receive` | Its copies appear in this device's history | On after pairing | No |
| `clipboard.apply` | Its copies are written into this device's system clipboard | On, except for content flagged as commands | Multi-line text that looks like a shell command goes to history only until the user picks it |
| `files.send` | Send files (they land in a quarantine folder and never open on their own) | On after pairing | Over a size threshold |
| `input.control` | Drive this device's keyboard and mouse | **Off** | Yes, on first use each session; a visible indicator while active; a local hotkey ends it at once |
| `ports.connect` | Reach ports this device has explicitly shared | **Off** | Sharing a port is a local action on this device |
| `tasks.run` | Run tasks this device has **defined locally** (never arbitrary commands) | **Off** | Yes, per task, with an "always allow" option the user can revoke |
| `search.query` | Query this device's local index | Off | No (read-only, results filtered by sensitivity) |
| `agent.read` / `agent.act` | Let an AI assistant read context or request actions | **Off** | `agent.act` always needs a human confirmation |

## Engineering practices

- **Structure:** `tether-core` stays pure (no I/O), so protocol and security logic run in the deterministic simulator. Cryptography, storage, networking, OS hooks and each feature live in separate crates with narrow public APIs.
- **Testing:** unit tests; `proptest` for merge and decode properties; `cargo-fuzz` targets for every decoder and parser (wire messages, manifests, content detection); simulator invariants for security properties (for example, "an operation from a revoked device is never applied").
- **CI gates (required to merge):** fmt, clippy `-D warnings`, tests, the simulator, `cargo deny` (advisories, licences, duplicate and banned crates), `cargo audit`, `npm audit --omit=dev`, frontend typecheck, tests and build.
- **Dependencies:** each new dependency is justified in its PR. Lockfiles are committed. Third-party UI code is vendored and scanned before use (see `app/src/components/VENDORED.md`).
- **Releases:** signed binaries, published checksums, an SBOM, and reproducible builds where the toolchain allows.
- **Cryptography:** only reviewed libraries (rustls, `snow` for Noise, `ed25519-dalek`, `blake3`, RustCrypto's `chacha20poly1305`); no custom primitives.
- **Disclosure:** report vulnerabilities privately through GitHub security advisories on the repository. Fixes ship before details are published.

## Feature threat models

### Encrypted storage (P1.1)

```
Feature: the operation log and group root, saved in SQLite (crates/tether-daemon/src/store.rs).
Capability it needs: none (local only).
What an attacker gains if it is abused: the device's clipboard history, or a way to slip
  operations into the log without a peer's signature.
Who could abuse it: someone holding the disk or a backup of it; another program that can
  write files in the data folder; a crash at a bad moment.
Mitigations:
  - Each value is sealed with XChaCha20-Poly1305 (random 192-bit nonce) under a 256-bit key
    kept in the OS credential store next to the device key, never in a file or a log.
  - The associated data is the value's slot (table and row id, or meta name), so a sealed
    value moved to another row or table fails to open.
  - A sealed key-check value: a database sealed with another key is moved aside, not
    half-read, and the daemon starts empty.
  - Every operation is re-verified on load (limits, signature, membership), as if a peer sent it.
  - WAL with synchronous=FULL: each append and rewrite is one atomic, durable transaction.
  - Exclusive locking: a second daemon on the same profile is refused.
  - The P0 plain-text file is imported once, then overwritten with zeros and deleted.
Limits: a stored operation larger than the wire frame limit is dropped without being opened.
Test or invariant proving the main property: store::tests (nothing_readable_reaches_the_disk,
  a_tampered_row_is_dropped_and_the_rest_load, a_sealed_value_cannot_be_moved_to_another_slot,
  a_database_sealed_with_another_key_is_set_aside, a_crash_mid_write_loses_nothing_committed,
  a_second_daemon_on_the_same_profile_is_refused, the_old_plain_text_store_is_imported_then_removed,
  the_key_is_never_printed).
Residual risk the user should know about: row count, sizes and timing are visible; history
  can be deleted or rolled back by whoever can write the file; malware running as the user
  can read the key from the credential store; zeroing the old plain-text file is best effort
  on SSDs and copy-on-write file systems.
```

### Local control channel (P1.2)

```
Feature: the channel the desktop app and the `tether` CLI use to drive the daemon
  (crates/tether-os/src/local.rs for the transport, crates/tether-ipc for the protocol,
  crates/tether-daemon/src/ipc.rs for the server).
Capability it needs: none; it is the user's own console. It can do anything the user can:
  read history, show a pairing code, join a group, revoke a device.
What an attacker gains if it is abused: everything. Opening a pairing window or joining a
  group the attacker controls hands over the clipboard of every device.
Who could abuse it:
  - another user account on the same machine;
  - a web page (the reason this is not a localhost TCP port: no DNS rebinding or
    cross-site requests can reach a named pipe or a Unix socket);
  - a sandboxed process running as the user (browser renderer, AppContainer app);
  - a program that grabs the pipe or socket name first and pretends to be the daemon,
    to steal the client's credentials or feed it false data;
  - a buggy or hostile client sending oversized or malformed frames.
Mitigations:
  - Windows: a named pipe whose name includes the user's SID, created with a protected
    DACL that grants access to the current user's SID only (no inherited entries), with
    FILE_FLAG_FIRST_PIPE_INSTANCE (if the name already exists, the daemon refuses to start
    rather than share it) and PIPE_REJECT_REMOTE_CLIENTS. Low-integrity and AppContainer
    processes are refused by the default mandatory label and by the DACL.
    Clients open the pipe with SECURITY_IDENTIFICATION, so a fake server cannot act as them.
  - Unix: a socket in a 0700 directory the user owns (checked, not assumed), mode 0600,
    and every connection's peer credentials must match the daemon's own user id.
  - Mutual authentication with a 256-bit token that the daemon makes fresh at every start
    and keeps in the OS credential store. Each side proves it knows the token with a keyed
    BLAKE3 MAC over both sides' random nonces; the token itself never crosses the channel.
    A client that cannot prove it is closed without a reply; a server that cannot prove it
    is dropped by the client before it sends any request.
  - Before authentication the server sends only a protocol version and a random nonce.
  - Length-prefixed postcard frames with hard limits, strict decoding, and fail-closed
    handling: any bad frame closes the connection.
Limits: 8 clients at once; 5 s to authenticate; requests up to 320 KiB (16 KiB until Notes:
  a paste that fills a note must fit one request); replies up to 4 MiB;
  history pages of at most 100 items and about 2 MiB; 10 s to write a reply; 256 queued
  events per subscriber, after which the subscriber is told it lagged.
Test or invariant proving the main property:
  tether-os local::tests (the pipe's DACL holds one entry, for this user; a second server
  cannot take the name; on Unix the socket is 0600 in a 0700 directory);
  tether-ipc tests (a wrong or missing token gets nothing; a client refuses a server that
  does not know the token; oversized frames are refused);
  tether-daemon tests/control_e2e.rs (real daemons: wrong token gets nothing, too many
  clients are refused, and two daemons pair, join and sync driven only through the channel).
Residual risk the user should know about: any program running as the same user can read
  the token from the credential store and use the channel, just as it could read the
  clipboard directly; this is outside what an application can prevent. No test yet runs as
  a second OS user: the "other users are refused" property rests on the DACL and file
  modes, which the tests read back.
```

### `tether` CLI (P1.3)

```
Feature: the command-line client (crates/tether-cli): status, devices, history, watch,
  pair, join, revoke.
Capability it needs: none of its own; it is a client of the local control channel and can
  do only what that channel allows.
What an attacker gains if it is abused: through the CLI itself, little. The danger is in
  what it prints: clipboard text and device names chosen by another device. A terminal
  obeys escape sequences, so printed text could write the clipboard (OSC 52), retitle the
  window, move the cursor to rewrite earlier lines (hiding a held command or faking a
  prompt), or reorder text with bidi controls.
Who could abuse it: a paired device that is compromised (content); a program squatting the
  control endpoint (handled by the channel's mutual authentication).
Mitigations:
  - Every string that came from the daemon is passed through one sanitiser before printing:
    C0 and C1 control characters (ESC included) and DEL become visible symbols; bidi
    overrides, isolates and marks become U+FFFD; only newline is kept, and only in --full
    output. One-line previews also show newlines as a symbol.
  - Held items are labelled as held, with the reason.
  - Revoke asks for confirmation (unless --yes) and needs an unambiguous id prefix of at
    least 4 characters that names a current member, never this device.
  - The CLI never reads the database or the keys; it uses tether_ipc::Client, which refuses
    a server that cannot prove it knows the token.
Limits: those of the control channel; history is fetched in pages of at most 100.
Test or invariant proving the main property: tether-cli term::tests (escape sequences,
  OSC 52, C1 controls, bidi characters), and tether-daemon tests/cli_e2e.rs (a peer copies
  text containing an escape sequence; `tether history` and `tether watch` print no ESC byte).
Residual risk the user should know about: --full prints long text verbatim apart from
  control characters; a terminal with its own bugs in rendering ordinary Unicode is outside
  this tool's control.
```

### Clipboard change events and app privacy marks (P1.4)

```
Feature: the daemon learns about copies from the OS instead of polling every 100 ms
  (crates/tether-os/src/clipboard*, crates/tether-daemon/src/input.rs), and honours the marks
  that password managers and other apps put on private copies.
Capability it needs: none new (it reads this device's own clipboard, as before).
What an attacker gains if it is abused: a password copied from a password manager synced
  to every device and stored in history; or, through a bug in the OS glue, memory unsafety
  in the daemon.
Who could abuse it: any local app that writes the clipboard (it controls the content and
  the formats); a race between reading the content and reading its marks.
Mitigations:
  - Private marks are honoured on top of the pattern-based secret detection:
    Windows: ExcludeClipboardContentFromMonitorProcessing, Clipboard Viewer Ignore,
    CanIncludeInClipboardHistory = 0, CanUploadToCloudClipboard = 0 (the formats Windows'
    own clipboard history and cloud sync honour). If a flag is present but cannot be read,
    the copy is treated as private (fail closed).
    macOS: org.nspasteboard.ConcealedType and org.nspasteboard.TransientType.
    A marked copy is never put in the log, never synced, never stored.
  - Content and marks are read as one snapshot: the OS clipboard sequence number (Windows
    GetClipboardSequenceNumber, macOS changeCount) is read before and after; if it moved,
    the read is discarded and the next change event reads again. So a private copy cannot
    slip through under the marks of the copy before it.
  - All unsafe code lives in tether-os (a message-only window with AddClipboardFormatListener;
    reading a 4-byte flag under GlobalLock), each block with a SAFETY comment.
  - The daemon's own writes (applying a copy from a peer) are recognised and not echoed back.
Limits: events are coalesced: a burst of notifications for one copy causes one read.
Test or invariant proving the main property: tether-daemon input::tests (marks travel with
  the copy; the daemon's own writes are not echoed; order is kept between local copies and
  deliveries), ctx::tests (a marked copy is kept local and not logged); tether-os
  clipboard tests marked #[ignore] because they use the real clipboard (marks are detected,
  a change is reported, latency measured), run by hand on Windows.
Residual risk the user should know about: an app that marks nothing and whose content the
  pattern detector misses will still sync. Linux has no mark support yet (KDE's
  x-kde-passwordManagerHint is the candidate), and on Wayland and headless Linux the daemon
  falls back to polling every 100 ms.
```

### Reconnect with backoff (P1.5)

```
Feature: redialling trusted devices after a failure or a lost connection
  (crates/tether-core/src/backoff.rs for the schedule, crates/tether-daemon/src/net.rs for the
  loop, crates/tether-daemon/src/tls.rs for the QUIC keys that survive a restart).
Capability it needs: none new.
What an attacker gains if it is abused: a device that dials too eagerly can be used to
  amplify traffic or drain a laptop's battery; one that gives up too easily can be kept
  offline. New keys derived from the device key must not weaken the device key.
Who could abuse it: a peer or network that accepts and immediately drops connections; an
  on-path attacker forging QUIC stateless resets.
Mitigations:
  - Exponential backoff from 1 s to 30 s with equal jitter (each wait is uniform in
    [ceiling/2, ceiling]), so devices that lost the network together do not retry in lockstep.
  - A session shorter than 30 s counts as a failure, so accept-then-drop cannot cause a tight loop.
  - A target is never dialled twice at once (property-tested); `--peer` addresses go
    through the same schedule (before, they could pile up concurrent dials).
  - The stateless reset key and the connection ID key are derived from the device key with
    BLAKE3's KDF under their own context strings: one-way, so neither reveals the device key.
    Forging a reset needs the reset key; holding it allows only closing our connections,
    which an on-path attacker can already do by dropping packets. quinn rate-limits resets.
  - Keep-alive every 5 s, 15 s idle timeout: a vanished peer is noticed within 15 s.
Limits: at most one dial in flight per target; delays capped at 30 s; the loop re-checks at
  least every 30 s.
Test or invariant proving the main property: tether-core backoff::tests (ceilings, jitter
  bounds and spread as properties, no double dials under any sequence of events, flapping
  counts as failure); tether-daemon tests/reconnect_e2e.rs, #[ignore]d because it uses the
  credential store, run by hand (a hard-killed peer that comes back is reconnected in 0.9 s
  after a quick restart and 2.8 s after a 25 s outage).
Residual risk the user should know about: while a device is unreachable, its peers keep
  retrying every 15 to 30 s or so (a failed dial also waits out the 10 s handshake timeout).
```

### Desktop app: Tauri shell, tray and pause (P1.6)

```
Feature: the desktop app (app/src-tauri: window, tray, the page's commands) and the pause
  setting it drives (Request::SetPaused in tether-ipc; Ctx::set_paused in the daemon).
Capability it needs: none of its own. It is a client of the local control channel, like the
  CLI, and can do only what that channel allows. It starts tetherd if none is running.
What an attacker gains if it is abused: the page renders text chosen by other devices
  (clipboard items, device names). If that text could run script, or navigate the window to a
  remote page, the attacker would sit next to the app's commands: read history, open a
  pairing window, join a group the attacker controls, revoke devices. A link opened carelessly
  could launch a program (file:, ms-msdt:, search-ms:) or, for a network path, make Windows
  sign in to the attacker's host and leak the user's NTLM hash. A pause that silently
  resumed would sync copies the user meant to keep.
Who could abuse it: a compromised paired device (content, names); a web page the user is
  tricked into loading in the app; a program squatting the control channel (handled by its
  mutual authentication); a malicious npm or crate dependency.
Mitigations:
  - The page loads nothing remote. CSP: default-src 'none'; script-src 'self'; style-src
    'self' (the two vendored components that injected <style> were changed); img-src 'self'
    data:; font-src 'self'; connect-src ipc: only; no objects, frames, workers, forms or
    <base>. Object.prototype is frozen before the page loads (freezePrototype).
  - Text from other devices is rendered as text by React; the only HTML inserted is
    highlight.js output, which escapes its input.
  - The window can show only the bundled page: every navigation elsewhere is refused
    (on_navigation, guard::is_app_url), and new windows (window.open, target=_blank) are
    refused. The native file-drop handler is off.
  - The page's whole API is the app's own commands, listed in build.rs (an app manifest), so
    Tauri refuses any command a window's capability does not grant. Each window has its own
    capability (capabilities/main.json, capabilities/palette.json), which adds only event
    listening and window theme/hide, and applies to local content only. Plugins (notification,
    global shortcut, single instance) are used from Rust; none of their commands is granted.
    No file system, shell, HTTP, clipboard or opener plugin is exposed to the page.
  - Every argument is checked in Rust before it reaches the daemon: device ids parse as
    keys, pairing codes are six digits, addresses parse as ip:port (64 bytes at most),
    history pages are clamped, unknown fields in a cursor are refused.
  - open_link opens only http(s) and mailto links, at most 2048 bytes, without control
    characters or user:password@, and only on a click in the page. "Show in File Explorer"
    is not offered, because a network path would leak the NTLM hash.
  - The app holds no keys, no history and no sockets; tetherd stays the only privileged
    process (rule 9). It finds tetherd next to its own executable, never on PATH.
  - A daemon the app starts runs with --managed: no console on stdin, and a clean shutdown
    (connections closed, control token removed) when stdin closes, which happens when the app
    quits or crashes. A daemon the user started by hand is used as is and never stopped.
  - The daemon's log goes to a file (Tether/logs/tetherd-<profile>.log). Log lines never hold
    clipboard text: held items are logged without their preview.
  - Pause is stored sealed in the encrypted database; an unreadable flag counts as paused
    (fail closed). While paused, local copies are not recorded and peers' copies are not
    written to the system clipboard; they still arrive in history.
  - Single instance (default profile): a second launch focuses the running window.
Limits: those of the control channel; history pages of 100; device list polled every 3 s
  only while the window is visible.
Test or invariant proving the main property: tether-app guard::tests (only the bundled page
  is a navigation target; file:, UNC, javascript:, ms-msdt:, search-ms: and credentialed
  links are refused), view::tests (codes, addresses, ids and cursors from the page are
  checked); tether-daemon ctx::tests::while_paused_nothing_is_recorded_or_applied_and_history_still_fills,
  store::tests::the_pause_flag_persists_and_a_tampered_one_reads_as_paused,
  input::tests::log_lines_never_carry_clipboard_text, control_e2e (a paused device sends and
  applies nothing until resumed; a managed daemon stops cleanly when its stdin closes).
  Checked by hand in a release build on Windows through the WebView2 DevTools protocol: the
  served CSP header matches the config; the page loads with no violations; an injected inline
  <style> or <script>, an inline event handler, string eval from page code, a remote fetch,
  image or iframe are each blocked with a violation report; five commands outside the list
  (opener, window close, fs, shell, event emit) are refused by the ACL; navigating to a remote
  page and window.open are refused; killing the app makes its daemon shut down cleanly and
  remove its control token. Not automated yet (no UI tests in CI).
Residual risk the user should know about: the CSP keeps img-src data: for one vendored
  texture; links the user clicks open in their browser, where the usual web risks apply;
  the webview itself (WebView2, WKWebView, WebKitGTK) is part of the trusted base and is
  updated by the OS, not by Tether.
```

### Open at sign-in (P1.6 follow-up)

```
Feature: a per-user login entry that starts the app hidden in the tray
  (app/src-tauri/src/login.rs; the autostart / set_autostart commands).
Capability it needs: none. It writes only the current user's own login entry: the
  HKCU Run key on Windows, a launch agent on macOS, an XDG autostart file on Linux.
What an attacker gains if it is abused: code run at every sign-in. A command line with an
  unquoted path containing a space ("C:\Users\Ann Lee\...\tether-app.exe") makes Windows
  try C:\Users\Ann.exe first, so whoever can create that file runs at sign-in (CWE-428).
Who could abuse it: a local program that can write where an unquoted path would resolve;
  a page trying to turn the entry on behind the user's back.
Mitigations:
  - Off until the user turns it on with the switch on the Overview; the page can call only
    our two commands (the autostart plugin, whose library writes paths unquoted, is not used).
  - The path is quoted on Windows and in the Linux desktop entry. A Linux path with
    characters a desktop entry would need to escape (" ` $ \ %, newlines) is refused rather
    than half-escaped. macOS launch agents take an argument array (no quoting needed).
  - The entry runs this executable with --hidden and, for a non-default profile, --profile;
    one entry per profile.
  - A hidden start does not ask for focus (focusing a window shows it).
Limits: one entry per profile.
Test or invariant proving the main property: tether-app login::tests (a path with a space
  is quoted; refused characters), main::tests (the entry's arguments parse back to a hidden
  start on the same profile). Checked by hand on Windows: the Run value written, read back
  and removed; a --hidden launch shows no window.
Residual risk the user should know about: malware running as the user can rewrite the
  entry, as it can any per-user setting.
```

### Pins and deletes (P2.1)

```
Feature: pin, unpin and delete clipboard items on every device (OpKind::Pin and
  OpKind::Delete in tether-core; Request::SetPinned / Request::Delete on the control
  channel; the app's set_pinned and delete_item commands).
Capability it needs: none new: any active member may pin or delete any item, as it may copy.
What an attacker gains if it is abused: a compromised member can hide history from every
  device, or pin junk. Aimed at copies it has not seen yet, it could hide another device's
  future copies as they arrive.
Who could abuse it: a compromised paired device; anyone forging operations on the network.
Mitigations:
  - Pins and deletes are signed operations like any other: forged, tampered or unknown-author
    ones are refused (the simulator's adversary injects forged deletes of real items).
  - Each names its target by (device, seq) and a BLAKE3 hash of the item's text
    (domain "tether/item/v1"). One whose hash does not match the held item does nothing,
    so a device cannot aim a delete at a copy it has not seen.
  - Resolution is a pure function of the operations held (OpLog::marks): a delete is a
    tombstone and always wins; a pin is last-writer-wins by (HLC time, device, seq). Every
    device that holds the same operations shows the same history.
  - Revocation applies as for copies: a revoked device's later pins and deletes are refused.
  - Deleted items are never delivered to the system clipboard and never sent to the app.
Limits: those of any operation (MAX_OPS_PER_MESSAGE, peer rate limits).
Test or invariant proving the main property: tether-core log::tests (delete hides an item
  even if it arrives first; delete beats a concurrent or later pin; the latest pin wins in
  any arrival order; a wrong hash does nothing; a revoked device's later delete does not
  count); tether-sim check_marks (every honest device's deletes and pins equal an oracle
  rebuilt from what the devices did, under faults, partitions, forged deletes and
  revocation; 10,000 seeds); control_e2e::pins_and_deletes_reach_every_device. Mutation
  checks: making the oldest pin win, or letting a pin outrank a delete, fails the simulator.
Residual risk the user should know about: a delete hides the item; its encrypted operation
  stays in each device's database until compaction (P2) removes it. A delete is final:
  there is no undo. A compromised member can still delete everything it has seen, until
  it is revoked.
```

### Device names (P2, quick win QW1)

```
Feature: rename a device on every device (OpKind::Rename; Request::Rename; `tether rename`;
  the app's rename_device command and the editable name on the Devices page).
Capability it needs: none new: any active member may name any active member.
What an attacker gains if it is abused: confusion. A compromised member could swap names
  ("Desktop" ↔ "Laptop") so the user revokes the wrong device, or put control or
  direction characters in a name to spoof text in a terminal or the UI.
Who could abuse it: a compromised paired device; anyone forging operations.
Mitigations:
  - Renames are signed operations from members; forged ones are refused (the simulator's
    adversary forges renames; holds_forgery catches any that stick).
  - The latest rename wins by (HLC time, device, seq): every device shows the same name.
  - Names are display only and never used for trust. They are bounded (64 bytes, checked
    in core, the app and the daemon), and control and direction characters are stripped
    before a rename is signed and again before any name is shown; the CLI prints them
    through term::safe and the UI renders them as text.
  - Fingerprints (the start of each device's key) stay on every device card, and revoke
    names the device by id, so a misleading name cannot hide which key is affected.
  - A rename of a device that is not an active member is refused locally and ignored when
    resolved; a revoked device's later renames are refused like its other operations.
Limits: 64 bytes per name.
Test or invariant proving the main property: tether-core
  log::tests::the_latest_rename_wins_on_every_device_and_strangers_cannot_be_named;
  tether-sim check_marks (every honest device's names equal an oracle; forged renames are
  refused); control_e2e::a_rename_reaches_every_device_including_the_renamed_one (control
  characters stripped, empty names refused); tether-app view::tests (names bounded).
Residual risk the user should know about: a compromised member can still give devices
  misleading names until it is revoked; check the fingerprint before revoking.
```

### Notifications (P2, quick win QW2)

```
Feature: a native notification when a copy arrives from another device
  (app/src-tauri/src/notify.rs; the notifications / set_notifications commands; a switch on
  the Overview's "This device" card).
Capability it needs: none.
What an attacker gains if it is abused: clipboard text written to disk in plain text, because
  OS notification centres store what they show (Windows keeps it in the Action Center
  database); spoofed or misleading notifications; a flood of notifications.
Who could abuse it: a compromised paired device choosing text and its own name; a page
  calling the notification plugin directly.
Mitigations:
  - A notification never contains the copied text: it names the device and gives the length
    (and, for a held item, why it was held).
  - Device names in notifications are cleaned (no control or direction characters, 40
    characters at most); a blank name becomes "another device".
  - None for this device's own copies, while sync is paused, while the window is focused,
    or when the user turned them off; at most one every 3 seconds.
  - The notification plugin is used from Rust only; its commands are not granted to the page.
  - The settings file (whether to notify) is in the user's config folder and holds nothing
    secret; a missing, oversized or unreadable file means the defaults.
Limits: one notification per 3 s; settings file read only up to 64 KiB.
Test or invariant proving the main property: tether-app notify::tests
  (a_notification_never_carries_the_copied_text, no notification for own copies / paused /
  focused / off, names cleaned, bursts throttled, settings tolerate junk).
Residual risk the user should know about: the notification still reveals that a copy
  happened, when, from which device and how long it was, to anyone who can read the
  notification history.
```

### Send to one device (P2, quick win QW3)

```
Feature: put a history item on one other device's clipboard (OpKind::Send { item, to };
  Request::Send; the app's send_item command and "Send to <device>" buttons; Event::Received).
Capability it needs: clipboard.apply on the target, which paired devices already have:
  every copy from a member is already applied there. A send only picks which item.
What an attacker gains if it is abused: a compromised member could keep overwriting
  another device's clipboard, or try to get a command pasted by sending it on purpose;
  a captured send replayed later could overwrite the clipboard at a bad moment.
Who could abuse it: a compromised paired device; anyone replaying or forging operations.
Mitigations:
  - Sends are signed operations from members; forged ones are refused, and the log applies
    each (device, seq) once, so a replayed send is a duplicate and does nothing.
  - Only the named target acts on a send, and only while it is fresh: at most 2 minutes
    old and at most 30 s in the future by its HLC time (send_is_fresh). A send that
    arrives late (the target was offline) is not applied.
  - The item must be held and named by its content hash (as for pins and deletes), and not
    deleted; the sender cannot send to itself or to a non-member.
  - The receiving device applies its content policy as for any copy: a command-like or
    hidden-character item stays held in history, and nothing is applied while paused.
  - A send changes no shared state (history, pins, names), so it cannot affect convergence.
Limits: those of any operation; one clipboard write per batch of operations.
Test or invariant proving the main property: tether-core log::tests
  (a_send_is_for_its_target_only_and_only_for_a_live_item, sends_are_applied_only_while_fresh);
  control_e2e::sending_an_item_puts_it_on_that_devices_clipboard_only (applied on the target;
  a command stays held; nothing while paused; refused to self and for deleted items);
  tether-app notify::tests (a send's notification never shows the text).
Residual risk the user should know about: a compromised member can still put items it has
  seen on your clipboard (as it already could by copying them) until it is revoked.
```

### Quick paste palette and copy here (P2, quick win QW4)

```
Feature: a global shortcut (Alt+Shift+V) opens a small always-on-top window to search
  history and put an item on this device's clipboard (app/src/palette, palette window in
  main.rs, capabilities/palette.json); Request::CopyHere, which the main window's "Copy to
  this PC" also uses.
Capability it needs: none new; a local, explicit user action on this device.
What an attacker gains if it is abused: the palette sits over other apps and reads all
  history, so a script running in it could read or change everything; a global shortcut
  could swallow keys other apps need; writing the clipboard from the page would create a
  duplicate history item and need clipboard permission in the page.
Who could abuse it: content from a compromised peer rendered in the palette; another app
  holding the same shortcut.
Mitigations:
  - The palette is its own window with its own, narrower capability: status, devices,
    history and copy_here, plus hiding itself. It cannot pin, delete, send, rename, pair,
    revoke or open links. Same CSP and navigation guard as the main window; no new windows.
  - Item text is rendered as text; sensitive items are masked and their text is never
    searched.
  - Copy here goes through the daemon's clipboard thread, which knows its own writes, so the
    item is not recorded again and the page needs no clipboard permission. Held items can be
    copied here on purpose (an explicit local action, rule 3); the palette shows "held".
  - The shortcut is Alt+Shift+V, not Ctrl+Shift+V (used by other apps for "paste as plain
    text"). If another app holds it, registration fails, the app runs on, and the Overview
    says quick paste is off.
  - The palette hides when it loses focus, so it never lingers over other apps.
Limits: the latest 100 history items are searched; 50 results shown.
Test or invariant proving the main property: control_e2e::copy_here_puts_an_old_item_back_without_a_duplicate;
  input::tests (the clipboard thread does not echo its own writes); palette search tests
  (sensitive text never matched). Checked by hand in a release build through the DevTools
  protocol: the palette window's capability refuses pin, delete, send and rename.
Residual risk the user should know about: anyone at the unlocked computer can open the
  palette and read history, as they could open the main window.
```

### Notes and the scratchpad (P3, F11)

```
Feature: notes that several devices edit at once, offline included (crates/tether-core:
  notes.rs Fugue CRDT, delta.rs and live.rs for editors; daemon notes.rs; app notes.rs,
  modules/notes, scratchpad window). Design: docs/notes-design.md.
Capability it needs: none new. A note edit is a signed operation from a member, like a
  clipboard copy; it acts on nothing outside Tether.
What an attacker gains if it is abused: rewriting or hiding what the user wrote; filling
  every device's disk or memory with one huge note; slipping misleading text (direction
  overrides, escape codes) or a link into a note; a secret the user pasted reaching every
  device and staying in their history; a page script with access to every note.
Who could abuse it: a compromised member; a stranger forging edits; content written by a
  peer; the scratchpad window if its page were taken over; a buggy or hostile local client.
Mitigations:
  - Every edit is signed, verified and from a member; a revoked device's later edits are
    dropped everywhere and open editors are shown the rebuilt text (simulator, unit test).
  - Character ids are derived from the signed operation's seq and checked on decode, so a
    device cannot reuse an id to make devices disagree (`Edit::ids_belong_to`).
  - Limits: 4 KiB of text, 1,024 runs and ranges, 65,536 deleted characters per edit;
    256 KiB of text per note when typing (a note merged past that may shrink, not grow);
    a note over 1 MiB is not opened (a peer filling one on purpose closes it here); 4,096
    notes; titles 200 bytes and notebooks 64, cleaned of control and direction characters.
  - Typing is batched (one signed operation per 150 ms burst in the editor) and the daemon
    spaces signed edits at least 25 ms apart whatever a client sends.
  - Secrets: an edit that inserts something shaped like a key, token or private key (or a
    pasted password) is not recorded until the user answers "Keep it in the note", because
    notes go to every device and keep their history. A token finished key by key counts.
  - Shown as text: the editor is a plain textarea, and invisible characters (escape codes,
    direction overrides) are shown as visible stand-ins. The Markdown view renders React
    elements only, never HTML; links are buttons that go through open_link (http, https,
    mailto); its parser is bounded (no backtracking, inline markup read in the first 4,000
    characters of a line, nesting stops at 4).
  - Edit replay is harmless: each editor's edits are numbered and made once.
  - The scratchpad window has its own capability: two commands that take no note name
    (scratch_open, scratch_edit), plus moving and hiding itself. It receives only the
    scratchpad's events. The scratchpad's name is not one an operation can create.
  - Nothing typed in a note reaches a log file (checked in notes_e2e).
Limits: as above; history lists the newest 500 revisions.
Test or invariant proving the main property: simulator check_notes (convergence, nothing lost,
  live copy equals the log, editors that lag behind equal the note, every version in the
  history is complete, metadata against an oracle, forged edits never stick); tether-core
  notes, delta, live tests (property tests, including exact change reporting); daemon
  notes::tests and tests/notes_e2e.rs (two real daemons; secrets wait for the user; full
  note and pacing; nothing logged); app notes::tests (ids and deltas from the page are
  checked; the scratchpad window hears about the scratchpad only); page tests (session
  against a stand-in daemon with lag, faults and restarts; Markdown on hostile input).
Residual risk the user should know about: deleting text from a note does not remove it
  from the log: it stays in every device's (encrypted) history until compaction (P2.2).
  The secret check is pattern-based. A member can still write anything into a shared note,
  as it can copy anything; revoking it removes what it wrote after the cutoff. Pausing sync
  pauses the clipboard, not notes.
```

### Pulse: heartbeats, device health and wake-up (P3, F10)

```
Feature: every connected device sends a heartbeat every 2 s as a QUIC datagram carrying its
  health (CPU, memory, home-disk space, battery, uptime, OS name, Tether version, and the
  hardware addresses of adapters on a private network); a phi-accrual failure detector reads
  the arrival times; the Devices page shows it and can wake a sleeping device with a
  Wake-on-LAN packet (crates/tether-core/src/pulse.rs, tether-os health.rs, daemon pulse.rs).
Capability it needs: none new; health is shared only with members, as clipboard items are.
  Waking is a local user action on this device.
What an attacker gains if it is abused: activity tracking (when each machine is on, how
  busy); a flood of datagrams to burn CPU; a wake packet aimed at a machine that is not the
  user's; false health shown on a card; a "low storage" scare.
Who could abuse it: someone on the network; a compromised member; a local client.
Mitigations:
  - Beats ride the existing mutually authenticated QUIC connection (nothing is sent to or
    accepted from a stranger), and are taken only from active members.
  - Never stored or put in the log; health is kept in memory, latest value only. What is
    saved (sealed in the database) is how to wake each member: its adapter addresses and
    when it was last heard from.
  - Strict decoding: at most 1 KiB, exact postcard, every value within limits (CPU <= 100 %,
    used <= total, battery <= 100 %, at most 4 addresses, OS name 48 bytes, cleaned).
  - A peer's beats beyond 4 a second are dropped; the datagram buffer is 64 KiB.
  - Health is display only: nothing is decided from it except where to send a wake packet.
  - Wake: only for an active member, only to addresses that device reported about itself
    over its authenticated connection (zero, broadcast, multicast and locally administered
    addresses are dropped), at most once per 5 s per device, sent as a LAN broadcast to
    ports 9 and 7. A wake packet carries no data and cannot do anything but power a machine
    on; a lying member can at most wake some other machine on the user's LAN.
  - Health values never reach a log file (checked in pulse_e2e).
  - Sampling is cheap and adaptive: every 2 s only while someone has the Devices page open
    (on any device), every 30 s otherwise.
Limits: as above.
Test or invariant proving the main property: tether-core pulse tests (decoder property-fuzzed
  on hostile bytes; limits; phi never decreases with silence, never NaN; regular beats stay
  live and a stopped device is called down within 10 s; magic packet layout; which addresses
  are wakeable); daemon pulse::tests (strangers' beats ignored, floods and junk dropped, low
  storage told once a day, waking only for members with an address and not too often, the
  wake book survives a restart, this device's own reading is clamped and cleaned);
  tests/pulse_e2e.rs (two real daemons see each other's health; a killed peer is called down
  in 7.6 s, before the 15 s QUIC idle timeout); app notify tests.
Residual risk the user should know about: every member sees every member's health and when it
  is on. Wake-on-LAN must be enabled on the sleeping machine's adapter (often only for wired
  Ethernet), and works only on the same network. Battery is not read on macOS yet; macOS and
  Linux readings are unverified on real machines.
```

## Threat model template (one per feature)

```
Feature:
Capability it needs:
What an attacker gains if it is abused:
Who could abuse it: (network / server / compromised peer / content / agent / dependency)
Mitigations: (rules and capability defaults above, plus anything specific)
Limits: (sizes, rates, timeouts)
Test or invariant proving the main property:
Residual risk the user should know about:
```
