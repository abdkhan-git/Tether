# Notes (F11): design

Notes that merge character by character across your computers, including after hours offline, without losing or scrambling either side's edits. This note fixes the design before code; SECURITY.md gets the threat model when it ships.

## What has to be true

1. **Convergence.** Two devices that hold the same operations show the same text, whatever order the operations arrived in.
2. **Nothing lost.** Every insert that was not deleted is in the text; every delete removes exactly what it named.
3. **No interleaving.** If the desktop types "hello" and the laptop types "world" at the same spot while apart, the result is "helloworld" or "worldhello", never "hweolrllod". This is where most text CRDTs fail in practice, and the reason for the choice below.
4. **Fast.** A keystroke shows instantly; a remote edit appears within the sync latency (milliseconds on a LAN); a 100 KB note opens instantly.
5. **Fits Tether's log.** Edits are signed operations from members, verified, revoked with their author, stored encrypted, and replicated by the same version-vector sync. No second sync system.

## The CRDT: Fugue

Text is a sequence CRDT: every character gets a permanent id, and positions are expressed relative to other characters' ids, never as indexes.

**Choice: Fugue** (Weidner and Kleppmann, "The Art of the Fugue", 2023), tree form.

- RGA, the classic choice, interleaves text typed *backwards* concurrently. Logoot/LSEQ interleave in more cases. Fugue was designed to avoid interleaving in both directions, and the paper proves it. Yjs/YATA avoids most interleaving but its rules are harder to state and to check.
- The tree form is a short, checkable rule set: easy to implement, property-test and put in the simulator.

**Structure.** Each character is a node with an id, a parent (another character, or the root) and a side (left or right of the parent). The text is an in-order walk: left children, the node, right children. Siblings on the same side of the same parent are ordered by id. Deleted characters stay as tombstones, so positions that refer to them still resolve.

**Inserting between `a` and `b`** (neighbours in the visible text, or the start/end): if `a` has no right children, the new character becomes `a`'s right child; otherwise it becomes `b`'s left child. That one rule is what keeps concurrent runs apart.

**Ids.** `(counter, device)`: the author's device id and a counter that is **derived from the signed operation carrying the character**: `seq × 2^20 + offset`. Ids are therefore unique by construction, and any device can check from the operation alone that its ids belong to it (`Edit::ids_belong_to`). A free per-device counter would let a compromised device reuse an id for two different characters, and devices that saw the two in different orders would disagree. Ordered by counter, then device, so every device sorts siblings the same way.

## Operations

A keystroke must not be a signed operation: at 27 µs to sign and 84 µs to verify, and 160+ bytes each, that would cost more than the text. Typing is **batched**: the daemon collects edits for up to 150 ms or 4 KiB, then emits one signed operation.

```
OpKind::NoteCreate { title }                          // the note's id is this op's (device, seq)
OpKind::NoteEdit   { note, inserts: [Run], deletes: [IdRange] }
OpKind::NoteMeta   { note, title?, notebook?, pinned?, trashed? }   // latest wins per field
```

- A `Run` is a run of characters typed in a row: `{ first: counter, parent, side, text }`. The first character attaches to `(parent, side)`; each next one is the previous one's right child. "hello" typed in place is one run, not five inserts.
- `IdRange` names deleted characters compactly: `{ device, from, to }` (counters), so deleting a typed word is one range.
- A run whose parent has not arrived yet (it came from a third device) is held aside and attached when the parent arrives. Because the result depends only on the set of operations, this cannot make devices disagree.

**Limits** (checked on decode, like every operation): 4 KiB of inserted text and 1,024 ranges per edit; 256 KiB of visible text per note; 4,096 notes. Edits beyond a limit are refused locally with a clear message; refused remotely as malformed.

## Where the CRDT runs, and how the editor talks to it

The CRDT lives in `tether-core::notes` (pure Rust, no I/O), so the daemon and the simulator run the same code. The daemon keeps each open note's tree in memory and updates it edit by edit as operations arrive; it rebuilds from the log on load or after a membership change prunes operations. **The rebuild must cover every note any held edit names (`OpLog::note_refs` + `build_note`), not only notes whose create has arrived:** edits from one device can arrive before the create from another, and a rebuild that skipped them would silently drop those edits. The simulator found exactly this bug (6 seeds in 1,000) before the daemon existed.

The editor in the page works in **indexes** (the textarea's world), not ids. Between the page and the daemon:

- The page holds `(text, version)` from the daemon.
- It sends edits as `{ version, at: index, delete: n, insert: "..." }`.
- If remote edits arrived since `version`, the daemon **rebases** the index over them before mapping it to ids. There is only one editor per note per device, so this is the simple two-party case of operational transformation (shift the index by inserts and deletes before it), not the general one.
- The daemon replies with the new version, and pushes remote changes as index patches so the page can keep the cursor where the user is.

This keeps the page free of CRDT code and WebAssembly (which the CSP would otherwise have to allow) and makes the daemon the only place that signs.

## History and "who wrote what"

Both come from the operations already kept:

- **Who wrote what:** every character's id names its author's device. The editor can colour text by device, and history can show each edit's author and time.
- **Version history:** the note as it was at time `t` is the CRDT built from its operations with HLC time ≤ `t`. Restoring a version emits a normal edit: delete what is not in the old version, insert again what is missing (as new characters).

## Compaction

Tombstones and old operations grow without bound. Compaction (P2.2) will let a device replace a note's operations with a signed snapshot once every active member is known to hold them (a stable version vector). History older than the snapshot is then gone; the retention for that is a user setting. Until compaction ships, the limits above bound the growth.

## Simulator invariants

Added to `tether-sim` alongside the clipboard checks:

- Devices create notes and type, delete and paste at random positions while partitioned, offline and revoked.
- **Convergence:** every honest device's text and authorship for every note is identical.
- **Live copy equals the log:** each device keeps notes in memory, applying edits in whatever order the faulty network delivered them, exactly as the daemon will; at the end that copy must equal a rebuild from the log. (Ordering siblings by arrival instead of id fails 236 of 300 seeds; without this check the simulator could not see it.)
- **Nothing lost:** the text equals an oracle built from the inserts and deletes the devices actually made.
- **No interleaving:** a run each device typed concurrently at the same position appears contiguous in the result.
- Forged and tampered edits never take effect.

## Out of scope for the first version

The whiteboard (shapes and strokes need their own merge rules), rich text beyond Markdown source, comments, and sharing notes with other people (Spaces, P8).
