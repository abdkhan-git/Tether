# Tether security model

Tether runs on every computer a person owns, can type into them, and holds what they copy, including passwords. A flaw here harms the user directly, so security is a requirement on every change, not a feature to add later.

No one can promise software with zero vulnerabilities. This document makes a narrower promise that we can keep: a small attack surface, defence in depth, explicit rules that every change follows, and checks in CI that enforce them.

## Current status (2026-11-23)

The stage 1 daemon is a prototype and is **not safe on shared networks**. Known issues, all fixed by phase P0 in [ROADMAP.md](ROADMAP.md):

| ID | Issue | Impact |
| --- | --- | --- |
| V1 | Any host on the LAN can connect; certificates are not verified and clients are not authenticated (`AcceptAnyCert` in `tether-daemon/src/net.rs`). | A stranger on the same Wi-Fi can request and receive the full clipboard history. |
| V2 | Received text is written to the system clipboard with no policy. | Clipboard injection: a malicious command waits for the next paste into a terminal. |
| V3 | Device identity is self-asserted in `Hello`; operations are unsigned. | Impersonation of a paired device; forged history; hijacking a peer slot. |
| V4 | No limits on connections, streams, message count or text size. | Memory exhaustion (denial of service). |
| V5 | Sensitivity detection exists only in the UI. | Copied secrets are synced. |

Until P0 lands, run `tetherd` only with `--stdin` on one machine, or on a network with no other users.

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
- **Cryptography:** only reviewed libraries (rustls, `snow` for Noise, `ed25519-dalek`, `blake3`); no custom primitives.
- **Disclosure:** report vulnerabilities privately through GitHub security advisories on the repository. Fixes ship before details are published.

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
