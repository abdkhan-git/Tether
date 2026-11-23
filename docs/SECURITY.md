# Tether security model

Tether runs on every computer a person owns, can type into them, and holds what they copy, including passwords. A flaw here harms the user directly, so security is a requirement on every change, not a feature to add later.

No one can promise software with zero vulnerabilities. This document makes a narrower promise that we can keep: a small attack surface, defence in depth, explicit rules that every change follows, and checks in CI that enforce them.

## Current status (2026-11-23)

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
Limits: 8 clients at once; 5 s to authenticate; requests up to 16 KiB; replies up to 4 MiB;
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
