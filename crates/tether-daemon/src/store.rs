//! Durable state: the group root and an append-only file of signed operations.
//!
//! Stage 2 replaces this with SQLite. Until then the format is simple and
//! crash-safe: length-prefixed records, `fsync` after every append, and a
//! torn final record (crash mid-write) is truncated on load. Every record is
//! re-verified when loaded (signature, limits, membership), so editing the
//! file cannot inject operations: at worst it loses some.
//!
//! The file holds clipboard history in plain text, protected by the user's
//! account permissions. Encryption at rest with a key from the credential
//! store comes with the SQLite move (ROADMAP P1).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tether_core::limits::{op_within_limits, MAX_FRAME};
use tether_core::{DeviceId, Op};

pub struct Store {
    dir: PathBuf,
    ops: File,
}

pub struct Loaded {
    pub store: Store,
    pub root: Option<DeviceId>,
    pub ops: Vec<Op>,
    /// Records dropped because they were torn or malformed.
    pub dropped: usize,
}

fn open_private(path: &Path, append: bool) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.create(true).read(true);
    if append {
        o.append(true);
    } else {
        o.write(true).truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
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

/// Write a file atomically: temp file, fsync, rename.
fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = open_private(&tmp, false)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

fn encode_record(op: &Op) -> Vec<u8> {
    let body = postcard::to_stdvec(op).expect("operations always serialise");
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

impl Store {
    pub fn open(dir: &Path) -> Result<Loaded> {
        create_private_dir(dir).with_context(|| format!("creating {}", dir.display()))?;
        let root = match fs::read_to_string(dir.join("root")) {
            Ok(s) => s.trim().parse::<DeviceId>().ok(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("reading the group root"),
        };

        let path = dir.join("ops.log");
        let mut bytes = Vec::new();
        match File::open(&path) {
            Ok(mut f) => {
                f.read_to_end(&mut bytes)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("reading the operation log"),
        }

        let (mut ops, mut good, mut dropped) = (Vec::new(), 0usize, 0usize);
        let mut at = 0usize;
        while at + 4 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes")) as usize;
            let end = at + 4 + len;
            if len > MAX_FRAME || end > bytes.len() {
                break; // torn or corrupt tail
            }
            match postcard::from_bytes::<Op>(&bytes[at + 4..end]) {
                Ok(op) if op_within_limits(&op) => ops.push(op),
                _ => dropped += 1,
            }
            at = end;
            good = at;
        }
        if good < bytes.len() {
            dropped += 1;
            let f = OpenOptions::new().write(true).open(&path)?;
            f.set_len(good as u64)?; // drop the torn tail so appends stay aligned
            f.sync_all()?;
        }

        let file = open_private(&path, true).context("opening the operation log")?;
        Ok(Loaded { store: Store { dir: dir.to_path_buf(), ops: file }, root, ops, dropped })
    }

    pub fn set_root(&mut self, root: DeviceId) -> Result<()> {
        replace(&self.dir.join("root"), root.to_hex().as_bytes()).context("saving the group root")
    }

    pub fn append(&mut self, op: &Op) -> Result<()> {
        self.ops.write_all(&encode_record(op))?;
        self.ops.sync_data().context("syncing the operation log")
    }

    /// Replace the whole log (after a revocation pruned operations).
    pub fn rewrite<'a>(&mut self, ops: impl Iterator<Item = &'a Op>) -> Result<()> {
        let bytes: Vec<u8> = ops.flat_map(encode_record).collect();
        let path = self.dir.join("ops.log");
        replace(&path, &bytes).context("rewriting the operation log")?;
        self.ops = open_private(&path, true)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tether_core::{DeviceKey, OpKind, OpLog};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tether-store-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn round_trips_and_survives_a_torn_write() {
        let dir = temp_dir("torn");
        let mut log = OpLog::new(DeviceKey::for_test(1));
        let mut loaded = Store::open(&dir).unwrap();
        for i in 0..3 {
            let op = log.append_local(OpKind::ClipboardText(format!("item {i}")), 1_000 + i).unwrap();
            loaded.store.append(&op).unwrap();
        }
        loaded.store.set_root(log.me()).unwrap();
        drop(loaded);
        // Simulate a crash halfway through writing a fourth record.
        let mut f = OpenOptions::new().append(true).open(dir.join("ops.log")).unwrap();
        f.write_all(&[200, 0, 0, 0, 1, 2, 3]).unwrap();
        drop(f);

        let loaded = Store::open(&dir).unwrap();
        assert_eq!(loaded.ops.len(), 3);
        assert_eq!(loaded.root, Some(log.me()));
        assert_eq!(loaded.dropped, 1);
        let mut fresh = OpLog::new(DeviceKey::for_test(1));
        assert_eq!(fresh.apply_batch(loaded.ops, 2_000).applied, 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tampered_record_is_refused_when_replayed() {
        let dir = temp_dir("tamper");
        let mut log = OpLog::new(DeviceKey::for_test(1));
        let mut loaded = Store::open(&dir).unwrap();
        let mut op = log.append_local(OpKind::ClipboardText("original".into()), 1_000).unwrap();
        op.kind = OpKind::ClipboardText("edited on disk".into());
        loaded.store.append(&op).unwrap();
        drop(loaded);
        let loaded = Store::open(&dir).unwrap();
        let mut fresh = OpLog::new(DeviceKey::for_test(1));
        let out = fresh.apply_batch(loaded.ops, 2_000);
        assert_eq!((out.applied, out.rejected), (0, 1));
        let _ = fs::remove_dir_all(&dir);
    }
}
