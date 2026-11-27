# Drop (F4): the shared shelf — design

Drag a file, folder, screenshot, link or text onto a shelf on one computer; open the shelf on another and drag it out. No destination to choose, nothing to start. This note fixes the design before code; SECURITY.md gets the threat model when it ships. It lives in the **Files** page (today's Transfers, renamed; shared folders, F12, join it later) and a small floating shelf on a shortcut, not a new tab.

## Decided with the owner (2026-11-26)

- **Fetching:** items up to 64 MB come over in the background as soon as they appear (instant and available offline); bigger ones come when taken out.
- **Expiry:** an unpinned item tidies itself away after 7 days, on every device; pinned items stay.
- **Taking out:** files go to **Downloads\Tether** (marked as downloaded); dragging out puts them wherever they are dropped.
- **Quota (default, adjustable later):** the shelf store keeps at most 10 GB or 10 % of the disk, whichever is smaller, and never fetches below the low-storage line.

## What has to be true

1. **Every device shows the same shelf**, whatever order things arrived in, after removals, pins and expiry.
2. **The log stays small.** A 10 GB file must not put 10 GB, or even its chunk list, into an operation that every device keeps forever.
3. **Nothing received runs or opens by itself**, and no name a peer chose can write outside the place Tether puts files (SECURITY.md rules 3 and 6).
4. **Resumable and never re-sent.** After a pulled cable, a crash or a reboot, a transfer continues with only the missing pieces; a piece the receiver already holds (from any item) is never fetched again. Roadmap target: a 10 GB item resumes after a cable pull with no chunk re-sent.
5. **Encrypted at rest**, like the log: no file content reaches the disk in the clear until the user takes it out.
6. **Bounded:** a member cannot fill another device's disk.

## The model: a small record in the log, the bytes outside it

```
OpKind::ShelfAdd    { kind, name, size, root: [u8; 32], expires: Option<ms> }   // the item is this op's (device, seq)
OpKind::ShelfRemove { item: ShelfRef }                                          // removal wins, like a clipboard delete
OpKind::ShelfPin    { item: ShelfRef, pinned: bool }                            // latest wins; a pinned item never expires
```

- **`root`** is the BLAKE3 hash of the item's **manifest**: for a file, its size and ordered chunk hashes; for a folder, its entries (relative path, size, chunk hashes). The manifest is itself content-addressed data, fetched and verified by `root` like any chunk, so the operation is about 200 bytes whatever the item's size. `ShelfRef` names an item by `(device, seq)` and `root`, so a removal cannot be aimed at something its author never saw (as `ItemRef` does for the clipboard).
- **Text and links** up to 64 KiB travel inline in the operation instead of as chunks, and go through the same content policy as the clipboard (secrets stay on the device; commands are shown, never run).
- **Chunks:** FastCDC content-defined chunking (`fastcdc` crate; 256 KiB min, 1 MiB average, 4 MiB max), BLAKE3 per chunk. Content-defined boundaries mean an edited file shares most chunks with the old one.
- **Expiry is computed, not an operation:** an unpinned item is gone `expires` after its HLC time on every device that holds the same operations, with no extra writes.

The shelf is a function of the operations held (adds minus removed or expired, pins latest-wins), checked by the simulator against an oracle like pins and deletes.

## Moving the bytes

- A device that wants an item asks its connected peers for the manifest, then for the chunks it lacks, on their own QUIC streams (`Message::GetChunk { hash }` → the chunk, or "not here"), never inside the sync stream: clipboard and notes never wait behind a file.
- Each chunk is verified against its hash before it is stored; a peer that sends a wrong one is dropped from that transfer.
- A bounded window of chunks in flight per peer (16), so memory stays flat for any size. With several peers holding the item, chunks come from whichever answers fastest (Pulse already measures them).
- **Resumable by construction:** received chunks are in the store; after any interruption the receiver asks again for exactly what is missing.
- Peers serve only chunks of items that are on the shelf (not removed, not expired), only to active members.

## Storage and garbage collection

- **Chunk store:** `%LOCALAPPDATA%\Tether\<profile>\chunks\`, one file per chunk named by its hash, each sealed with XChaCha20-Poly1305 under the database key with the hash as associated data (a chunk moved or swapped fails to open). Identical content from any item is stored once.
- **Garbage collection is local:** a chunk no live item references is deleted. Removal, expiry and revocation all just make items not live. This is why Drop does **not** need log compaction (P2.2) first: the bulky part never enters the log, and a shelf operation is ~200 bytes (50 items a day is ~3.6 MB a year of log). Compaction stays its own P2 item.
- **Quota:** the shelf store has a cap (default below); fetching refuses to go past it or below the low-storage line Pulse uses (5 % or 5 GB free), and says so.

## Taking an item out

- The user drags it out of the window or presses "Save". The file is assembled from verified chunks into a temporary file, flushed, then renamed into place (atomic), in **Downloads\Tether** by default.
- **Names are rebuilt, not trusted:** each path component is cleaned (no `..`, separators, drive letters, colons or alternate data streams, control or direction characters, Windows reserved names such as `CON` or `NUL`, trailing dots or spaces) and bounded; a folder is written only beneath its own new directory.
- On Windows the file gets the **Mark of the Web** (`Zone.Identifier`), so SmartScreen and Office treat it as downloaded. Tether never opens it; "Show in File Explorer" only reveals it.
- Drag-out from the window uses a native drag (Tauri's drag plugin) of the assembled file. Dropping files *in* uses Tauri's native drop handled in Rust: the page learns only names and sizes, never gets file-system access.

## The floating shelf

Like the scratchpad: a small always-on-top window on a shortcut (Alt+Shift+D), with its own narrow capability (list, add from a drop, take out), showing the latest items.

## Simulator invariants

- Devices add, remove, pin and let items expire while partitioned and under attack; every honest device shows the same shelf, equal to an oracle over what the devices did; forged removals never take effect.
- A chunk model: devices hold random subsets of chunks; a device that fetches an item through faulty, reordering links ends up with every chunk exactly once and nothing that fails its hash (a forged chunk injected by the adversary is never stored).

## Build order

1. Core: shelf operations, the shelf as a function of the log, chunking and manifests; simulator invariants.
2. Daemon: chunk store (sealed), serving and fetching chunks with resume, garbage collection, quota; protocol and app commands.
3. Files page: the shelf, drop in, take out, progress from real transfers (the page's sample data goes).
4. Floating shelf; pins and expiry in the UI; measured transfer speed and the cable-pull resume test.

## Out of scope for the first version

Shared folders kept in sync (F12 Drive), sending to someone else (Spaces), streaming a video while it downloads, and fetching from devices on another network (P5 relay).
