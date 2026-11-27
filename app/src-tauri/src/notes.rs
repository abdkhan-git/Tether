//! Notes: the page's commands, what it sees, and checks on what it sends.
//!
//! The main window can work with any note. The scratchpad window has two
//! commands of its own (`scratch_open`, `scratch_edit`) that take no note name,
//! and its capability grants nothing else: it can reach the scratchpad note
//! and no other. Threat model: docs/SECURITY.md, "Notes".

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;
use tether_core::notes::{Span, MAX_NOTEBOOK_BYTES, MAX_NOTE_BYTES, MAX_TITLE_BYTES};
use tether_core::{Delta, NoteRef, Step, Timestamp};
use tether_ipc as ipc;
use tether_ipc::{EditorId, Reply, Request};

use crate::link::Link;
use crate::view::device_id;
use crate::{unexpected, Res};

/// What the page calls the scratchpad.
const SCRATCHPAD: &str = "scratchpad";
/// Most steps in one edit from the page.
const MAX_STEPS: usize = 4096;

// --- What the page sees ---

/// A note's name on the page: its author and sequence number, or "scratchpad".
pub fn note_id(note: NoteRef) -> String {
    if note == NoteRef::SCRATCHPAD {
        SCRATCHPAD.to_string()
    } else {
        format!("{}:{}", note.device.to_hex(), note.seq)
    }
}

fn note_ref(id: &str) -> Res<NoteRef> {
    if id == SCRATCHPAD {
        return Ok(NoteRef::SCRATCHPAD);
    }
    let bad = || "that is not a note".to_string();
    let (device, seq) = id.split_once(':').ok_or_else(bad)?;
    if seq.is_empty() || !seq.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let seq: u64 = seq.parse().map_err(|_| bad())?;
    if seq == 0 {
        return Err(bad());
    }
    Ok(NoteRef { device: device_id(device).map_err(|_| bad())?, seq })
}

/// A delta as the page holds it: keep `n` as `n`, delete `n` as `-n`, insert
/// as the text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StepView {
    Count(i64),
    Text(String),
}

pub type DeltaView = Vec<StepView>;

pub fn delta_view(delta: &Delta) -> DeltaView {
    delta
        .steps()
        .iter()
        .map(|s| match s {
            Step::Keep(n) => StepView::Count(i64::from(*n)),
            Step::Delete(n) => StepView::Count(-i64::from(*n)),
            Step::Insert(t) => StepView::Text(t.clone()),
        })
        .collect()
}

fn delta(steps: DeltaView) -> Res<Delta> {
    if steps.len() > MAX_STEPS {
        return Err("that edit has too many parts".into());
    }
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        out.push(match step {
            StepView::Text(t) => Step::Insert(t),
            StepView::Count(n) => {
                let count = u32::try_from(n.unsigned_abs()).map_err(|_| "that edit does not fit the note")?;
                if n < 0 {
                    Step::Delete(count)
                } else {
                    Step::Keep(count)
                }
            }
        });
    }
    let delta = Delta::from_steps(out);
    if delta.inserted_bytes() > MAX_NOTE_BYTES {
        return Err(format!("a note holds at most {} KiB of text", MAX_NOTE_BYTES >> 10));
    }
    Ok(delta)
}

/// An editor's name: 32 hex digits the page picked at random.
fn editor_id(hex: &str) -> Res<EditorId> {
    let bad = || "that is not an editor id".to_string();
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let mut out = [0u8; 16];
    for (byte, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| bad())?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| bad())?;
    }
    Ok(out)
}

pub fn editor_hex(id: &EditorId) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

fn label(text: &str, max: usize, what: &str) -> Res<String> {
    let text = text.trim();
    if text.len() > max {
        return Err(format!("{what} is at most {max} bytes"));
    }
    Ok(text.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct SpanView {
    device: String,
    chars: u32,
}

pub fn spans(authors: &[Span]) -> Vec<SpanView> {
    authors.iter().map(|s| SpanView { device: s.device.to_hex(), chars: s.chars }).collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummary {
    id: String,
    title: String,
    notebook: String,
    pinned: bool,
    trashed: bool,
    created: u64,
    updated: u64,
    updated_by: String,
    preview: String,
    chars: u32,
}

impl From<ipc::NoteSummary> for NoteSummary {
    fn from(n: ipc::NoteSummary) -> Self {
        Self {
            id: note_id(n.note),
            title: n.title,
            notebook: n.notebook,
            pinned: n.pinned,
            trashed: n.trashed,
            created: n.created.wall_ms,
            updated: n.updated.wall_ms,
            updated_by: n.updated_by.to_hex(),
            preview: n.preview,
            chars: n.chars,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NoteText {
    version: u64,
    text: String,
    authors: Vec<SpanView>,
    answered: u64,
}

impl From<ipc::NoteText> for NoteText {
    fn from(t: ipc::NoteText) -> Self {
        Self { version: t.version, authors: spans(&t.authors), text: t.text, answered: t.answered }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Revision {
    author: String,
    /// When its first and last edit were made. `at` with `counter` and
    /// `author` names the revision.
    first: u64,
    at: u64,
    counter: u32,
    edits: u32,
    inserted: u32,
    deleted: u32,
}

impl From<tether_core::NoteRevision> for Revision {
    fn from(r: tether_core::NoteRevision) -> Self {
        Self {
            author: r.author.to_hex(),
            first: r.first.wall_ms,
            at: r.at.wall_ms,
            counter: r.at.counter,
            edits: r.edits,
            inserted: r.inserted,
            deleted: r.deleted,
        }
    }
}

/// What became of an edit.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "result", rename_all = "camelCase")]
pub enum Edited {
    Done,
    Stale,
    Secret { kind: String },
    Refused { reason: String },
}

/// An edit, as the page sends it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditView {
    base: u64,
    delta: DeltaView,
    editor: String,
    n: u64,
    allow_secret: bool,
}

// --- Commands ---

async fn open(link: &Link, note: NoteRef, editor: &str) -> Res<NoteText> {
    let editor = Some(editor_id(editor)?);
    match link.call(Request::NoteOpen { note, editor }, true).await? {
        Reply::NoteText(t) => Ok(t.into()),
        r => Err(unexpected(r)),
    }
}

async fn edit(link: &Link, note: NoteRef, edit: EditView) -> Res<Edited> {
    let req = Request::NoteEdit(ipc::NoteEdit {
        note,
        base: edit.base,
        delta: delta(edit.delta)?,
        editor: editor_id(&edit.editor)?,
        n: edit.n,
        allow_secret: edit.allow_secret,
    });
    // Safe to repeat: the daemon makes each of an editor's edits once.
    match link.call(req, true).await? {
        Reply::NoteEdited => Ok(Edited::Done),
        Reply::NoteStale => Ok(Edited::Stale),
        Reply::NoteSecret { kind } => Ok(Edited::Secret { kind }),
        Reply::NoteRefused { reason } => Ok(Edited::Refused { reason }),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
pub async fn notes(link: State<'_, Arc<Link>>) -> Res<Vec<NoteSummary>> {
    match link.call(Request::Notes, true).await? {
        Reply::Notes(list) => Ok(list.into_iter().map(NoteSummary::from).collect()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
pub async fn note_create(link: State<'_, Arc<Link>>, title: String, notebook: String) -> Res<String> {
    let req = Request::NoteCreate {
        title: label(&title, MAX_TITLE_BYTES, "a title")?,
        notebook: label(&notebook, MAX_NOTEBOOK_BYTES, "a notebook name")?,
    };
    match link.call(req, false).await? {
        Reply::NoteCreated { note } => Ok(note_id(note)),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
pub async fn note_open(link: State<'_, Arc<Link>>, note: String, editor: String) -> Res<NoteText> {
    open(&link, note_ref(&note)?, &editor).await
}

#[tauri::command]
pub async fn note_edit(link: State<'_, Arc<Link>>, note: String, edit: EditView) -> Res<Edited> {
    self::edit(&link, note_ref(&note)?, edit).await
}

#[tauri::command]
pub async fn note_meta(
    link: State<'_, Arc<Link>>,
    note: String,
    title: Option<String>,
    notebook: Option<String>,
    pinned: Option<bool>,
    trashed: Option<bool>,
) -> Res<()> {
    let req = Request::NoteMeta {
        note: note_ref(&note)?,
        title: title.map(|t| label(&t, MAX_TITLE_BYTES, "a title")).transpose()?,
        notebook: notebook.map(|n| label(&n, MAX_NOTEBOOK_BYTES, "a notebook name")).transpose()?,
        pinned,
        trashed,
    };
    match link.call(req, true).await? {
        Reply::NoteChanged => Ok(()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
pub async fn note_history(link: State<'_, Arc<Link>>, note: String) -> Res<Vec<Revision>> {
    match link.call(Request::NoteHistory { note: note_ref(&note)? }, true).await? {
        Reply::NoteHistory(list) => Ok(list.into_iter().map(Revision::from).collect()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
pub async fn note_at(link: State<'_, Arc<Link>>, note: String, at: u64, counter: u32, author: String) -> Res<NoteText> {
    let at = Timestamp { wall_ms: at, counter };
    let req = Request::NoteAt { note: note_ref(&note)?, at, author: device_id(&author)? };
    match link.call(req, true).await? {
        Reply::NoteText(t) => Ok(t.into()),
        r => Err(unexpected(r)),
    }
}

#[tauri::command]
pub async fn note_restore(link: State<'_, Arc<Link>>, note: String, at: u64, counter: u32, author: String) -> Res<()> {
    let at = Timestamp { wall_ms: at, counter };
    let req = Request::NoteRestore { note: note_ref(&note)?, at, author: device_id(&author)? };
    match link.call(req, true).await? {
        Reply::NoteChanged => Ok(()),
        r => Err(unexpected(r)),
    }
}

/// The scratchpad window's way in: the scratchpad note, and no other.
#[tauri::command]
pub async fn scratch_open(link: State<'_, Arc<Link>>, editor: String) -> Res<NoteText> {
    open(&link, NoteRef::SCRATCHPAD, &editor).await
}

#[tauri::command]
pub async fn scratch_edit(link: State<'_, Arc<Link>>, edit: EditView) -> Res<Edited> {
    self::edit(&link, NoteRef::SCRATCHPAD, edit).await
}

/// Is this event one the scratchpad window needs? Nothing about any other
/// note, or about the clipboard, goes to it.
pub fn for_scratchpad(event: &ipc::Event) -> bool {
    match event {
        ipc::Event::NotePatch { note, .. } | ipc::Event::NoteNoop { note, .. } => *note == NoteRef::SCRATCHPAD,
        ipc::Event::Lagged { .. } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tether_core::notes::TextChange;
    use tether_core::DeviceKey;

    #[test]
    fn note_names_round_trip_and_reject_junk() {
        let note = NoteRef { device: DeviceKey::for_test(3).id(), seq: 42 };
        assert_eq!(note_ref(&note_id(note)).unwrap(), note);
        assert_eq!(note_ref("scratchpad").unwrap(), NoteRef::SCRATCHPAD);
        let device = note.device.to_hex();
        for bad in ["", ":", "scratchpad:1", &device, &format!("{device}:"), &format!("{device}:0"), "zz:1"] {
            assert!(note_ref(bad).is_err(), "{bad:?} accepted");
        }
        assert!(note_ref(&format!("{device}:+1")).is_err());
        assert!(note_ref(&format!("{device}:99999999999999999999")).is_err());
        // No spelling of the scratchpad's own name gets in by the other route.
        assert!(note_ref(&format!("{}:0", "0".repeat(64))).is_err());
    }

    #[test]
    fn deltas_round_trip_and_are_bounded() {
        let d = Delta::splice(3, 2, "é\"x");
        let json = serde_json::to_string(&delta_view(&d)).unwrap();
        assert_eq!(json, r#"[3,"é\"x",-2]"#);
        assert_eq!(delta(serde_json::from_str(&json).unwrap()).unwrap(), d);
        assert!(delta(vec![StepView::Count(i64::MIN)]).is_err());
        assert!(delta(vec![StepView::Count(1 << 40)]).is_err());
        assert!(delta(vec![StepView::Text("x".repeat(MAX_NOTE_BYTES + 1))]).is_err());
        assert!(delta(vec![StepView::Count(1); MAX_STEPS + 1]).is_err());
        assert!(serde_json::from_str::<DeltaView>("[null]").is_err());
        assert!(serde_json::from_str::<DeltaView>("[1.5]").is_err());
    }

    #[test]
    fn editor_ids_are_32_hex_digits() {
        let id = editor_id("000102030405060708090a0b0c0d0e0f").unwrap();
        assert_eq!(id[15], 15);
        assert_eq!(editor_hex(&id), "000102030405060708090a0b0c0d0e0f");
        for bad in ["", "00", &"g".repeat(32), &"é".repeat(16), &"0".repeat(33)] {
            assert!(editor_id(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn the_scratchpad_window_hears_about_the_scratchpad_only() {
        let patch = |note| ipc::Event::NotePatch { note, from: 1, to: 2, change: TextChange::default(), by: None };
        assert!(for_scratchpad(&patch(NoteRef::SCRATCHPAD)));
        assert!(!for_scratchpad(&patch(NoteRef { device: DeviceKey::for_test(1).id(), seq: 1 })));
        assert!(!for_scratchpad(&ipc::Event::HistoryChanged));
        assert!(!for_scratchpad(&ipc::Event::NotesChanged));
        assert!(for_scratchpad(&ipc::Event::Lagged { missed: 1 }));
    }
}
