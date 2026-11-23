//! Durable state, encrypted at rest.
//!
//! Each profile has one SQLite database, `tether.db`. Every value in it is
//! sealed with XChaCha20-Poly1305 under a 256-bit key kept in the OS
//! credential store next to the device key (`keystore.rs`). The files on disk
//! hold only ciphertext, row ids and SQLite's own bookkeeping.
//!
//! Integrity comes in layers:
//! - A value's slot (its table and row id, or its name in `meta`) is the
//!   AEAD's associated data, so a sealed value cannot be moved to another row
//!   or table: it fails to open there.
//! - Every operation is re-verified when loaded: limits here, then signature
//!   and membership in `OpLog::apply_batch`, exactly as if a peer had sent
//!   it. Deleting, replaying or corrupting rows can at worst lose operations,
//!   which peers send again.
//! - SQLite's write-ahead log with `synchronous=FULL` makes each append and
//!   each rewrite atomic and durable: after a crash the database holds the
//!   state from before the write or after it, never a mix.
//!
//! The database is opened in exclusive locking mode, so a second daemon on
//! the same profile fails to start instead of interleaving writes.
//!
//! Not hidden: how many rows there are, the size of each (roughly the length
//! of each copied text), and when the files changed.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use tether_core::limits::{op_within_limits, MAX_FRAME};
use tether_core::{DeviceId, Op};
use zeroize::Zeroizing;

const DB_FILE: &str = "tether.db";
const SCHEMA_VERSION: i64 = 1;
/// Domain separation for the associated data of every sealed value.
const DOMAIN: &[u8] = b"tether/store/v1";
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// A known value sealed into every database, so a wrong key is noticed once
/// at startup rather than as every row failing to open.
const CHECK: &[u8] = b"tether database key check";

/// The database key. Zeroed on drop and never printed.
pub struct DbKey(Zeroizing<[u8; 32]>);

impl DbKey {
    pub fn new(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }

    #[cfg(test)]
    pub fn for_test(n: u8) -> Self {
        Self(Zeroizing::new([n; 32]))
    }
}

impl fmt::Debug for DbKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DbKey(<redacted>)")
    }
}

/// Where a sealed value lives. Bound into its associated data.
#[derive(Clone, Copy)]
enum Slot {
    Meta(&'static str),
    Op(i64),
}

impl Slot {
    fn aad(self) -> Vec<u8> {
        let mut aad = DOMAIN.to_vec();
        match self {
            Slot::Meta(name) => {
                aad.extend_from_slice(b"\0meta\0");
                aad.extend_from_slice(name.as_bytes());
            }
            Slot::Op(id) => {
                aad.extend_from_slice(b"\0ops\0");
                aad.extend_from_slice(&id.to_le_bytes());
            }
        }
        aad
    }
}

/// Seals values as `nonce (24 bytes) || ciphertext || tag (16 bytes)`.
/// Nonces are random; at 192 bits, collisions are not a practical concern
/// however many values are written under one key.
struct Sealer(XChaCha20Poly1305);

impl Sealer {
    fn new(key: &DbKey) -> Self {
        Self(XChaCha20Poly1305::new_from_slice(key.0.as_ref()).expect("the key is 32 bytes"))
    }

    fn seal(&self, slot: Slot, plain: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("no secure randomness available: {e}"))?;
        let aad = slot.aad();
        let body = self
            .0
            .encrypt(&XNonce::from(nonce), Payload { msg: plain, aad: &aad })
            .map_err(|_| anyhow!("encryption failed"))?;
        let mut out = Vec::with_capacity(NONCE_LEN + body.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// `None` unless this key sealed this value for this slot.
    fn open(&self, slot: Slot, sealed: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
        if sealed.len() < NONCE_LEN + TAG_LEN {
            return None;
        }
        let (nonce, body) = sealed.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
        let aad = slot.aad();
        self.0.decrypt(&XNonce::from(nonce), Payload { msg: body, aad: &aad }).ok().map(Zeroizing::new)
    }
}

fn seal_op(sealer: &Sealer, id: i64, op: &Op) -> Result<Vec<u8>> {
    let plain = Zeroizing::new(postcard::to_stdvec(op).context("encoding an operation")?);
    sealer.seal(Slot::Op(id), &plain)
}

fn open_op(sealer: &Sealer, id: i64, sealed: &[u8]) -> Option<Op> {
    if sealed.len() > NONCE_LEN + MAX_FRAME + TAG_LEN {
        return None;
    }
    let plain = sealer.open(Slot::Op(id), sealed)?;
    postcard::from_bytes::<Op>(&plain).ok().filter(op_within_limits)
}

/// Insert `ops` with ids from `first`. Returns the next free id.
fn insert_ops<'a>(tx: &Transaction, sealer: &Sealer, first: i64, ops: impl IntoIterator<Item = &'a Op>) -> Result<i64> {
    let mut stmt = tx.prepare_cached("INSERT INTO ops (id, sealed) VALUES (?1, ?2)")?;
    let mut id = first;
    for op in ops {
        stmt.execute(params![id, seal_op(sealer, id, op)?])?;
        id += 1;
    }
    Ok(id)
}

pub struct Store {
    conn: Connection,
    sealer: Sealer,
    next_id: i64,
}

pub struct Loaded {
    pub store: Store,
    pub root: Option<DeviceId>,
    pub ops: Vec<Op>,
    /// Rows that failed to open or decode. They are deleted.
    pub dropped: usize,
    /// Operations imported from the plain-text log of earlier versions.
    pub imported: usize,
    /// Where a database sealed with a different key was moved.
    pub set_aside: Option<PathBuf>,
    /// Sync was paused when the daemon last ran. An unreadable flag counts
    /// as paused: failing closed keeps copies on this device until the user
    /// resumes.
    pub paused: bool,
}

impl Store {
    /// Open (or create) the database in `dir` and load everything in it.
    pub fn open(dir: &Path, key: &DbKey) -> Result<Loaded> {
        create_private_dir(dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(DB_FILE);
        let sealer = Sealer::new(key);
        let (conn, set_aside) = match connect(&path, &sealer)? {
            Some(conn) => (conn, None),
            None => {
                let aside = set_aside(&path)?;
                let conn = connect(&path, &sealer)?.ok_or_else(|| anyhow!("a new database failed its key check"))?;
                (conn, Some(aside))
            }
        };
        let next_id: i64 = conn.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM ops", [], |r| r.get(0))?;
        let mut store = Store { conn, sealer, next_id };
        let imported = import_legacy(dir, &mut store)?;
        let (root, ops, mut dropped) = store.load()?;
        let paused = match store.load_paused()? {
            Some(paused) => paused,
            None => {
                dropped += 1;
                true
            }
        };
        Ok(Loaded { store, root, ops, dropped, imported, set_aside, paused })
    }

    /// The saved pause flag: `Some(false)` if none was saved, `None` if it is unreadable.
    fn load_paused(&self) -> Result<Option<bool>> {
        let sealed: Option<Vec<u8>> =
            self.conn.query_row("SELECT sealed FROM meta WHERE name = 'paused'", [], |r| r.get(0)).optional()?;
        Ok(match sealed {
            None => Some(false),
            Some(sealed) => self.sealer.open(Slot::Meta("paused"), &sealed).and_then(|b| match b.as_slice() {
                [0] => Some(false),
                [1] => Some(true),
                _ => None,
            }),
        })
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<()> {
        let sealed = self.sealer.seal(Slot::Meta("paused"), &[u8::from(paused)])?;
        self.conn
            .execute(
                "INSERT INTO meta (name, sealed) VALUES ('paused', ?1)
                 ON CONFLICT (name) DO UPDATE SET sealed = excluded.sealed",
                [sealed],
            )
            .context("saving the pause setting")?;
        Ok(())
    }

    fn load(&mut self) -> Result<(Option<DeviceId>, Vec<Op>, usize)> {
        let mut dropped = 0;
        let sealed_root: Option<Vec<u8>> =
            self.conn.query_row("SELECT sealed FROM meta WHERE name = 'root'", [], |r| r.get(0)).optional()?;
        let root = sealed_root.and_then(|sealed| {
            let root = self.sealer.open(Slot::Meta("root"), &sealed).and_then(|b| postcard::from_bytes(&b).ok());
            dropped += usize::from(root.is_none());
            root
        });

        let (mut ops, mut bad) = (Vec::new(), Vec::new());
        {
            let mut stmt = self.conn.prepare("SELECT id, sealed FROM ops ORDER BY id")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let id: i64 = row.get(0)?;
                let sealed: Vec<u8> = row.get(1)?;
                match open_op(&self.sealer, id, &sealed) {
                    Some(op) => ops.push(op),
                    None => bad.push(id),
                }
            }
        }
        if !bad.is_empty() {
            let tx = self.conn.transaction()?;
            for id in &bad {
                tx.execute("DELETE FROM ops WHERE id = ?1", [id])?;
            }
            tx.commit().context("removing unreadable rows")?;
        }
        Ok((root, ops, dropped + bad.len()))
    }

    pub fn set_root(&mut self, root: DeviceId) -> Result<()> {
        let sealed = self.sealer.seal(Slot::Meta("root"), &postcard::to_stdvec(&root)?)?;
        self.conn
            .execute(
                "INSERT INTO meta (name, sealed) VALUES ('root', ?1)
                 ON CONFLICT (name) DO UPDATE SET sealed = excluded.sealed",
                [sealed],
            )
            .context("saving the group root")?;
        Ok(())
    }

    pub fn append(&mut self, op: &Op) -> Result<()> {
        self.append_all([op])
    }

    /// Append several operations in one transaction (one sync to disk).
    pub fn append_all<'a>(&mut self, ops: impl IntoIterator<Item = &'a Op>) -> Result<()> {
        let tx = self.conn.transaction()?;
        let next = insert_ops(&tx, &self.sealer, self.next_id, ops)?;
        tx.commit().context("saving operations")?;
        self.next_id = next;
        Ok(())
    }

    /// Replace every operation (after a revocation pruned some), atomically.
    pub fn rewrite<'a>(&mut self, ops: impl IntoIterator<Item = &'a Op>) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM ops", [])?;
        let next = insert_ops(&tx, &self.sealer, 1, ops)?;
        tx.commit().context("rewriting the operation log")?;
        self.next_id = next;
        Ok(())
    }
}

/// Open the database, creating the schema if needed. `None` means it was
/// sealed with a different key.
fn connect(path: &Path, sealer: &Sealer) -> Result<Option<Connection>> {
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    // Exclusive locking, set before WAL mode, keeps the WAL index in this
    // process's memory (no shared-memory file) and holds the lock until the
    // connection closes. Taking a write lock now makes a second daemon on
    // this profile fail here rather than later.
    conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .and_then(|()| conn.execute_batch("BEGIN EXCLUSIVE; COMMIT;"))
        .context("locking the database (is another tetherd running with this profile?)")?;
    conn.pragma_update(None, "synchronous", "FULL")?;

    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    match version {
        0 => conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS meta (name TEXT PRIMARY KEY, sealed BLOB NOT NULL) STRICT;
             CREATE TABLE IF NOT EXISTS ops (id INTEGER PRIMARY KEY, sealed BLOB NOT NULL) STRICT;
             PRAGMA user_version = 1;
             COMMIT;",
        )?,
        SCHEMA_VERSION => {}
        v => bail!("{} was written by a newer Tether (schema {v}); update Tether to open it", path.display()),
    }

    let check: Option<Vec<u8>> =
        conn.query_row("SELECT sealed FROM meta WHERE name = 'check'", [], |r| r.get(0)).optional()?;
    match check {
        Some(sealed) => {
            if sealer.open(Slot::Meta("check"), &sealed).as_deref().map(Vec::as_slice) != Some(CHECK) {
                return Ok(None);
            }
        }
        None => {
            let empty: bool =
                conn.query_row("SELECT NOT EXISTS (SELECT 1 FROM ops) AND NOT EXISTS (SELECT 1 FROM meta)", [], |r| {
                    r.get(0)
                })?;
            if !empty {
                return Ok(None); // rows without a key check: not ours to trust
            }
            conn.execute(
                "INSERT INTO meta (name, sealed) VALUES ('check', ?1)",
                [sealer.seal(Slot::Meta("check"), CHECK)?],
            )?;
        }
    }
    Ok(Some(conn))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = OsString::from(path.as_os_str());
    s.push(suffix);
    PathBuf::from(s)
}

/// Move a database we cannot read out of the way, keeping its bytes.
fn set_aside(path: &Path) -> Result<PathBuf> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let aside = with_suffix(path, &format!(".unreadable-{secs}"));
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let from = with_suffix(path, suffix);
        if from.exists() {
            fs::rename(&from, with_suffix(&aside, suffix))
                .with_context(|| format!("moving aside {}", from.display()))?;
        }
    }
    Ok(aside)
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

// --- One-time import of the plain-text store written by P0. Remove after P1. ---

const LEGACY_FILES: [&str; 4] = ["ops.log", "root", "ops.tmp", "root.tmp"];

/// Import `ops.log` and `root` into the database, then overwrite and delete
/// them. If the database already has operations, the files are leftovers of
/// an import that was interrupted after its commit, and are only deleted.
fn import_legacy(dir: &Path, store: &mut Store) -> Result<usize> {
    if !LEGACY_FILES.iter().any(|f| dir.join(f).exists()) {
        return Ok(0);
    }
    let mut imported = 0;
    if store.next_id == 1 {
        if let Ok(text) = fs::read_to_string(dir.join("root")) {
            if let Ok(root) = text.trim().parse::<DeviceId>() {
                store.set_root(root)?;
            }
        }
        let ops = read_legacy_ops(&dir.join("ops.log"))?;
        store.append_all(&ops)?;
        imported = ops.len();
    }
    for f in LEGACY_FILES {
        scrub_and_remove(&dir.join(f)).with_context(|| format!("removing the old plain-text file {f}"))?;
    }
    Ok(imported)
}

/// Length-prefixed postcard records; a torn or malformed tail is ignored.
fn read_legacy_ops(path: &Path) -> Result<Vec<Op>> {
    let mut bytes = Zeroizing::new(Vec::new());
    match File::open(path) {
        Ok(mut f) => {
            f.read_to_end(&mut bytes)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("reading the old operation log"),
    }
    let (mut ops, mut at) = (Vec::new(), 0usize);
    while at + 4 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes")) as usize;
        let end = at + 4 + len;
        if len > MAX_FRAME || end > bytes.len() {
            break;
        }
        if let Ok(op) = postcard::from_bytes::<Op>(&bytes[at + 4..end]) {
            if op_within_limits(&op) {
                ops.push(op);
            }
        }
        at = end;
    }
    Ok(ops)
}

/// Overwrite a file with zeros, sync, then delete it. Best effort: on SSDs
/// and copy-on-write file systems the old blocks may survive until reused.
fn scrub_and_remove(path: &Path) -> io::Result<()> {
    let len = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    {
        let mut f = OpenOptions::new().write(true).open(path)?;
        io::copy(&mut io::repeat(0).take(len), &mut f)?;
        f.sync_all()?;
    }
    fs::remove_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tether_core::{DeviceKey, OpKind, OpLog};

    fn temp_dir(name: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tether-store-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn copies(log: &mut OpLog, texts: &[&str]) -> Vec<Op> {
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| log.append_local(OpKind::ClipboardText(t.to_string()), 1_000 + i as u64).unwrap())
            .collect()
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// Every byte of every file in `dir`.
    fn all_bytes(dir: &Path) -> Vec<u8> {
        fs::read_dir(dir).unwrap().flat_map(|e| fs::read(e.unwrap().path()).unwrap_or_default()).collect()
    }

    fn raw(dir: &Path) -> Connection {
        Connection::open(dir.join(DB_FILE)).unwrap()
    }

    #[test]
    fn round_trips_ops_and_root() {
        let dir = temp_dir("roundtrip");
        let key = DbKey::for_test(1);
        let mut log = OpLog::new(DeviceKey::for_test(1));
        let ops = copies(&mut log, &["one", "two", "three"]);
        {
            let mut l = Store::open(&dir, &key).unwrap();
            l.store.append(&ops[0]).unwrap();
            l.store.append_all(&ops[1..]).unwrap();
            l.store.set_root(log.me()).unwrap();
        }
        let l = Store::open(&dir, &key).unwrap();
        assert_eq!((l.ops.len(), l.root, l.dropped, l.set_aside.is_none()), (3, Some(log.me()), 0, true));
        let mut fresh = OpLog::new(DeviceKey::for_test(1));
        assert_eq!(fresh.apply_batch(l.ops, 2_000).applied, 3);
        drop(l.store);

        // A rewrite replaces everything and keeps appends working after it.
        let mut l = Store::open(&dir, &key).unwrap();
        l.store.rewrite(&ops[..1]).unwrap();
        l.store.append(&ops[2]).unwrap();
        drop(l);
        assert_eq!(Store::open(&dir, &key).unwrap().ops, vec![ops[0].clone(), ops[2].clone()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pause_flag_persists_and_a_tampered_one_reads_as_paused() {
        let dir = temp_dir("paused");
        let key = DbKey::for_test(1);
        assert!(!Store::open(&dir, &key).unwrap().paused, "a new profile is not paused");
        Store::open(&dir, &key).unwrap().store.set_paused(true).unwrap();
        assert!(Store::open(&dir, &key).unwrap().paused);
        Store::open(&dir, &key).unwrap().store.set_paused(false).unwrap();
        assert!(!Store::open(&dir, &key).unwrap().paused);

        // Flip one byte of the sealed flag: it no longer opens, so it fails closed.
        let conn = raw(&dir);
        let mut sealed: Vec<u8> =
            conn.query_row("SELECT sealed FROM meta WHERE name = 'paused'", [], |r| r.get(0)).unwrap();
        *sealed.last_mut().unwrap() ^= 1;
        conn.execute("UPDATE meta SET sealed = ?1 WHERE name = 'paused'", [sealed]).unwrap();
        drop(conn);
        let l = Store::open(&dir, &key).unwrap();
        assert_eq!((l.paused, l.dropped), (true, 1));
        drop(l);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_readable_reaches_the_disk() {
        let dir = temp_dir("at-rest");
        let key = DbKey::for_test(0x5a);
        let marker = "correct horse battery staple 7f3a91";
        let mut log = OpLog::new(DeviceKey::for_test(1));
        let ops = copies(&mut log, &[marker]);
        let mut l = Store::open(&dir, &key).unwrap();
        l.store.append_all(&ops).unwrap();
        l.store.set_root(log.me()).unwrap();
        // Check while open (data sits in the write-ahead log) and after close.
        let check = |when: &str| {
            let bytes = all_bytes(&dir);
            assert!(bytes.len() > 4096, "nothing written ({when})");
            assert!(!contains(&bytes, marker.as_bytes()), "plain text on disk ({when})");
            assert!(!contains(&bytes, &[0x5a; 32]), "the key reached the disk ({when})");
            assert!(!contains(&bytes, &postcard::to_stdvec(&log.me()).unwrap()), "root in clear ({when})");
        };
        check("open");
        drop(l);
        check("closed");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tampered_row_is_dropped_and_the_rest_load() {
        let dir = temp_dir("tamper");
        let key = DbKey::for_test(2);
        let ops = copies(&mut OpLog::new(DeviceKey::for_test(1)), &["keep", "flip"]);
        Store::open(&dir, &key).unwrap().store.append_all(&ops).unwrap();
        {
            let db = raw(&dir);
            let mut sealed: Vec<u8> = db.query_row("SELECT sealed FROM ops WHERE id = 2", [], |r| r.get(0)).unwrap();
            let last = sealed.len() - 1;
            sealed[last] ^= 1;
            db.execute("UPDATE ops SET sealed = ?1 WHERE id = 2", [sealed]).unwrap();
        }
        let l = Store::open(&dir, &key).unwrap();
        assert_eq!((l.ops, l.dropped), (vec![ops[0].clone()], 1));
        drop(l.store);
        assert_eq!(Store::open(&dir, &key).unwrap().dropped, 0, "the bad row was deleted");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sealed_value_cannot_be_moved_to_another_slot() {
        let dir = temp_dir("swap");
        let key = DbKey::for_test(3);
        let mut log = OpLog::new(DeviceKey::for_test(1));
        let ops = copies(&mut log, &["a", "b"]);
        {
            let mut l = Store::open(&dir, &key).unwrap();
            l.store.append_all(&ops).unwrap();
            l.store.set_root(log.me()).unwrap();
        }
        {
            let db = raw(&dir);
            // Swap the two ops, and put the root's sealed value into an op row.
            db.execute_batch(
                "UPDATE ops SET id = -1 WHERE id = 1; UPDATE ops SET id = 1 WHERE id = 2; UPDATE ops SET id = 2 WHERE id = -1;
                 INSERT INTO ops (id, sealed) SELECT 3, sealed FROM meta WHERE name = 'root';
                 INSERT INTO ops (id, sealed) SELECT 4, sealed FROM meta WHERE name = 'check';",
            )
            .unwrap();
        }
        let l = Store::open(&dir, &key).unwrap();
        assert_eq!((l.ops.len(), l.dropped, l.root), (0, 4, Some(log.me())));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_database_sealed_with_another_key_is_set_aside() {
        let dir = temp_dir("wrong-key");
        let ops = copies(&mut OpLog::new(DeviceKey::for_test(1)), &["secret"]);
        Store::open(&dir, &DbKey::for_test(4)).unwrap().store.append_all(&ops).unwrap();

        let l = Store::open(&dir, &DbKey::for_test(5)).unwrap();
        let aside = l.set_aside.clone().expect("set aside");
        assert!(l.ops.is_empty() && aside.exists());
        drop(l);
        // The original bytes are intact and still open with the right key.
        fs::rename(&aside, dir.join(DB_FILE)).unwrap();
        assert_eq!(Store::open(&dir, &DbKey::for_test(4)).unwrap().ops, ops);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crash_mid_write_loses_nothing_committed() {
        let (dir, crashed) = (temp_dir("crash"), temp_dir("crashed"));
        let key = DbKey::for_test(6);
        let ops = copies(&mut OpLog::new(DeviceKey::for_test(1)), &["one", "two", "three"]);
        let mut l = Store::open(&dir, &key).unwrap();
        l.store.append(&ops[0]).unwrap();
        l.store.append_all(&ops[1..]).unwrap();
        // Snapshot the files as they are on disk while the daemon runs (the
        // commits live in the write-ahead log, not yet checkpointed), then
        // tear the log's tail as a crash during the next write would.
        fs::create_dir_all(&crashed).unwrap();
        for f in [DB_FILE.to_string(), format!("{DB_FILE}-wal")] {
            fs::write(crashed.join(&f), fs::read(dir.join(&f)).unwrap()).unwrap();
        }
        let mut wal = OpenOptions::new().append(true).open(crashed.join(format!("{DB_FILE}-wal"))).unwrap();
        wal.write_all(&[0xA5; 1500]).unwrap();
        drop(wal);
        drop(l);

        let l = Store::open(&crashed, &key).unwrap();
        assert_eq!((l.ops, l.dropped), (ops, 0));
        let _ = (fs::remove_dir_all(&dir), fs::remove_dir_all(&crashed));
    }

    #[test]
    fn a_second_daemon_on_the_same_profile_is_refused() {
        let dir = temp_dir("lock");
        let key = DbKey::for_test(7);
        let _first = Store::open(&dir, &key).unwrap();
        let second = Store::open(&dir, &key).err().expect("second open must fail");
        assert!(format!("{second:#}").contains("another tetherd"), "{second:#}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_old_plain_text_store_is_imported_then_removed() {
        let dir = temp_dir("legacy");
        let key = DbKey::for_test(8);
        let mut log = OpLog::new(DeviceKey::for_test(1));
        let ops = copies(&mut log, &["old one", "old two"]);
        fs::create_dir_all(&dir).unwrap();
        let mut file: Vec<u8> = Vec::new();
        for op in &ops {
            let body = postcard::to_stdvec(op).unwrap();
            file.extend_from_slice(&(body.len() as u32).to_le_bytes());
            file.extend_from_slice(&body);
        }
        file.extend_from_slice(&[200, 0, 0, 0, 1]); // torn tail
        fs::write(dir.join("ops.log"), file).unwrap();
        fs::write(dir.join("root"), log.me().to_hex()).unwrap();

        let l = Store::open(&dir, &key).unwrap();
        assert_eq!((l.imported, l.ops.clone(), l.root), (2, ops.clone(), Some(log.me())));
        assert!(!dir.join("ops.log").exists() && !dir.join("root").exists());
        drop(l);
        assert!(!contains(&all_bytes(&dir), b"old one"));
        let l = Store::open(&dir, &key).unwrap();
        assert_eq!((l.imported, l.ops), (0, ops));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_key_is_never_printed() {
        let shown = format!("{:?}", DbKey::for_test(0xab));
        assert_eq!(shown, "DbKey(<redacted>)");
    }
}
