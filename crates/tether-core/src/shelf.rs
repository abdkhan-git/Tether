//! Drop (F4): the shared shelf. Design: docs/drop-design.md.
//!
//! An item on the shelf is a small signed operation; its bytes live outside
//! the log as content-addressed chunks, described by a **manifest** whose
//! BLAKE3 hash (`root`) the operation carries. This module holds what every
//! device must agree on: the manifest format and its hash, the limits, the
//! names a received item may be written under, and the shelf as a function
//! of the operations held (see [`crate::OpLog::shelf`]). Pure: the daemon does
//! the chunking and the disk.

use serde::{Deserialize, Serialize};

use crate::DeviceId;

/// Text and links up to this size travel inside the operation.
pub const MAX_INLINE_BYTES: usize = 64 << 10;
/// Largest item.
pub const MAX_ITEM_BYTES: u64 = 256 << 30;
/// Longest item name, and longest path component inside a folder.
pub const MAX_NAME_BYTES: usize = 255;
/// Most files in a folder item, and most chunks in a manifest.
pub const MAX_ENTRIES: usize = 10_000;
pub const MAX_CHUNKS: usize = 262_144;
/// Largest encoded manifest (it travels as one message).
pub const MAX_MANIFEST_BYTES: usize = (3 << 20) + (512 << 10);
/// Content-defined chunk sizes (FastCDC): minimum, average, maximum.
pub const CHUNK_MIN: u32 = 256 << 10;
pub const CHUNK_AVG: u32 = 1 << 20;
pub const CHUNK_MAX: u32 = 4 << 20;
/// How long an unpinned item stays (decided with the owner: 7 days).
pub const DEFAULT_TTL_MS: u64 = 7 * 24 * 3600 * 1000;
/// Longest time to live an item may ask for.
pub const MAX_TTL_MS: u64 = 365 * 24 * 3600 * 1000;
/// Items up to this size are fetched as soon as they appear (decided with the owner).
pub const PREFETCH_MAX_BYTES: u64 = 64 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShelfKind {
    File,
    Folder,
    /// Text, inline in the operation.
    Text,
    /// A link, inline in the operation.
    Link,
}

/// A shelf item, as a removal or pin names it: the operation that added it,
/// and its root, so a removal cannot be aimed at an item its author never saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShelfRef {
    pub device: DeviceId,
    pub seq: u64,
    pub root: [u8; 32],
}

/// A piece of a file, named by the BLAKE3 hash of its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub hash: [u8; 32],
    pub len: u32,
}

/// One file of an item: its place (relative, `/`-separated; empty for a
/// single-file item), size and chunks in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub size: u64,
    pub chunks: Vec<ChunkRef>,
}

/// What an item's bytes are made of. Content-addressed: its hash is the item's root.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    TooLarge,
    Malformed,
    WrongRoot,
    OutOfLimits,
}

/// BLAKE3 of a chunk, as chunks are named.
pub fn chunk_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// The root of inline text: what a `ShelfRef` to a text item names.
pub fn text_root(text: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("tether/shelf-text/v1");
    h.update(text.as_bytes());
    *h.finalize().as_bytes()
}

impl Manifest {
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("manifests always serialise")
    }

    /// The item's root: a domain-separated hash of the encoded manifest.
    pub fn root(&self) -> [u8; 32] {
        root_of(&self.encode())
    }

    pub fn size(&self) -> u64 {
        self.entries.iter().map(|e| e.size).fold(0, u64::saturating_add)
    }

    /// Within limits: entry and chunk counts, sizes that add up, safe paths.
    pub fn within_limits(&self) -> bool {
        let chunks: usize = self.entries.iter().map(|e| e.chunks.len()).sum();
        self.entries.len() <= MAX_ENTRIES
            && chunks <= MAX_CHUNKS
            && self.size() <= MAX_ITEM_BYTES
            && self.entries.iter().all(|e| {
                e.chunks.iter().map(|c| u64::from(c.len)).sum::<u64>() == e.size
                    && e.chunks.iter().all(|c| c.len > 0 && c.len <= CHUNK_MAX)
                    && (e.path.is_empty() || safe_path(&e.path).is_some_and(|p| p.join("/") == e.path))
            })
    }

    /// Decode a manifest fetched from a peer: bounded, exact, the root it
    /// should have, and within limits.
    pub fn decode(bytes: &[u8], root: [u8; 32]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge);
        }
        if root_of(bytes) != root {
            return Err(ManifestError::WrongRoot);
        }
        let manifest: Manifest = match postcard::take_from_bytes(bytes) {
            Ok((m, [])) => m,
            _ => return Err(ManifestError::Malformed),
        };
        if !manifest.within_limits() {
            return Err(ManifestError::OutOfLimits);
        }
        Ok(manifest)
    }
}

fn root_of(encoded: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("tether/manifest/v1");
    h.update(encoded);
    *h.finalize().as_bytes()
}

/// Names Windows will not let a file have, whatever its extension.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹",
    "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²",
    "LPT³", "CONIN$", "CONOUT$",
];

/// A name a peer chose, rebuilt into one that is safe as a single file or
/// folder name on Windows, macOS and Linux: no separators, drive letters,
/// colons (alternate data streams), wildcards, control or direction
/// characters; not `.` or `..`; no reserved device name; no trailing dots
/// or spaces; at most [`MAX_NAME_BYTES`]. Never empty.
pub fn safe_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let bad = c.is_control()
            || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}');
        let c = if bad { '_' } else { c };
        if out.len() + c.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        out.push(c);
    }
    let mut out = out.trim_end_matches(['.', ' ']).trim_start().to_string();
    if out.is_empty() || out.chars().all(|c| c == '.') {
        out = "item".to_string();
    }
    let stem = out.split('.').next().unwrap_or("").trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        out.insert(0, '_');
        while out.len() > MAX_NAME_BYTES || out.ends_with(['.', ' ']) {
            out.pop();
        }
    }
    out
}

/// A relative path inside a folder item, as its safe components; `None` if
/// it is not one (empty, absolute, `.` or `..` parts, too deep, or a part
/// that would have to change to be safe).
pub fn safe_path(path: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = path.split('/').collect();
    if path.is_empty() || parts.len() > 32 {
        return None;
    }
    parts.into_iter().map(|p| (!p.is_empty() && safe_name(p) == p).then(|| p.to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn entry(path: &str, lens: &[u32]) -> Entry {
        let chunks: Vec<ChunkRef> = lens.iter().map(|&len| ChunkRef { hash: [len as u8; 32], len }).collect();
        Entry { path: path.into(), size: lens.iter().map(|&l| u64::from(l)).sum(), chunks }
    }

    #[test]
    fn a_manifest_is_named_by_its_root_and_checked() {
        let m = Manifest { entries: vec![entry("", &[1 << 20, 300])] };
        let bytes = m.encode();
        assert_eq!(Manifest::decode(&bytes, m.root()).unwrap(), m);
        assert_eq!(Manifest::decode(&bytes, [0; 32]), Err(ManifestError::WrongRoot));
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(Manifest::decode(&trailing, root_of(&trailing)), Err(ManifestError::Malformed));
        for bad in [
            Manifest { entries: vec![Entry { size: 5, ..entry("", &[3]) }] },
            Manifest { entries: vec![entry("", &[CHUNK_MAX + 1])] },
            Manifest { entries: vec![entry("", &[0])] },
            Manifest { entries: vec![entry("../evil.txt", &[3])] },
            Manifest { entries: vec![entry("a/CON/b", &[3])] },
            Manifest { entries: vec![entry("C:/x", &[3])] },
        ] {
            let bytes = bad.encode();
            assert_eq!(Manifest::decode(&bytes, root_of(&bytes)), Err(ManifestError::OutOfLimits), "{bad:?}");
        }
        assert!(Manifest { entries: vec![entry("photos/2026/a.jpg", &[10])] }.within_limits());
        assert_ne!(text_root("hello"), chunk_hash(b"hello"), "domains are kept apart");
    }

    #[test]
    fn names_from_a_peer_are_made_safe() {
        for (from, to) in [
            ("report.pdf", "report.pdf"),
            ("..", "item"),
            ("...", "item"),
            ("../../Windows/win.ini", ".._.._Windows_win.ini"),
            ("C:\\boot.ini", "C__boot.ini"),
            ("file.txt:Zone.Identifier", "file.txt_Zone.Identifier"),
            ("CON", "_CON"),
            ("nul.txt", "_nul.txt"),
            ("COM1 .log", "_COM1 .log"),
            ("trailing. . ", "trailing"),
            ("gnp.\u{202E}exe", "gnp._exe"),
            ("tab\there", "tab_here"),
            ("", "item"),
        ] {
            assert_eq!(safe_name(from), to, "{from:?}");
        }
        assert!(safe_name(&"é".repeat(400)).len() <= MAX_NAME_BYTES);
        assert_eq!(safe_path("a/b/c.txt").unwrap(), ["a", "b", "c.txt"]);
        for bad in ["", "/abs", "a//b", "a/../b", "a/./b", "a\\b", "a/b/", &"x/".repeat(40)] {
            assert!(safe_path(bad).is_none(), "{bad:?}");
        }
    }

    proptest! {
        /// Whatever a peer sends, the result is a single safe name.
        #[test]
        fn safe_names_are_always_safe(name in any::<String>()) {
            let n = safe_name(&name);
            prop_assert!(!n.is_empty() && n.len() <= MAX_NAME_BYTES);
            prop_assert!(n != "." && n != "..");
            prop_assert!(!n.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c)));
            prop_assert!(!n.ends_with('.') && !n.ends_with(' '));
            prop_assert_eq!(safe_name(&n), n.clone(), "already safe names stay as they are");
        }

        /// Hostile manifest bytes never panic the decoder.
        #[test]
        fn any_bytes_decode_or_fail_cleanly(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let _ = Manifest::decode(&bytes, root_of(&bytes));
        }
    }
}
