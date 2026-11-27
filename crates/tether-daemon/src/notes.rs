//! Notes: the ones open in an editor on this device, kept in memory, and what
//! the app may do with them. Design: docs/notes-design.md. Threat model:
//! docs/SECURITY.md, "Notes".
//!
//! The log is the truth. A note someone here has open is built from the log
//! once and then kept up to date edit by edit (`tether_core::live`, which the
//! simulator runs too); every other note is built when it is wanted. After a
//! change of membership, which can drop operations, open notes are rebuilt.
//!
//! Each change to an open note becomes an [`Event::NotePatch`], sent while the
//! state lock is held: subscribers see patches in the order they happened, and
//! an editor needs nothing else to stay in step.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use tether_core::content;
use tether_core::live::{LiveNote, Patch};
use tether_core::notes::{Doc, EditError, MAX_NOTEBOOK_BYTES, MAX_NOTES, MAX_NOTE_BYTES, MAX_TITLE_BYTES};
use tether_core::{Delta, DeviceId, NoteRef, NoteRevision, Op, OpKind, OpLog, Step, Timestamp};
use tether_ipc::{EditorId, Event, NoteEdit, NoteSummary, NoteText};

use crate::ctx::{clean_text, Ctx, State};

/// Most notes kept in memory for editors; the one used longest ago makes room.
const MAX_OPEN: usize = 8;
/// Largest note that can be opened. Edits made apart can take a note past
/// `MAX_NOTE_BYTES`, but a peer could also do it on purpose.
const MAX_OPEN_BYTES: usize = 4 * MAX_NOTE_BYTES;
/// Editors whose last answered edit is remembered.
const MAX_EDITORS: usize = 64;
/// Characters of a note shown in the list.
const PREVIEW_CHARS: usize = 160;
/// Signed edits are at least this far apart. Editors send a batch every 150 ms
/// or so; this holds for one that does not.
const EDIT_GAP: Duration = Duration::from_millis(25);
/// How far around an insertion to look for the rest of a secret (characters).
const SECRET_REACH: usize = 512;

pub struct NoteCache {
    open: HashMap<NoteRef, OpenNote>,
    /// The last version given to any note. It starts at a random place, so an
    /// editor's version from before a restart matches nothing.
    version: u64,
    /// Counts uses of open notes, to find the one used longest ago.
    uses: u64,
    /// The start and size of notes that have not changed since they were read.
    previews: HashMap<NoteRef, (String, u32)>,
    /// The last edit made for each editor, newest editor last.
    answered: VecDeque<(EditorId, u64)>,
    /// When the next edit may be made.
    next_turn: Instant,
}

struct OpenNote {
    live: LiveNote,
    used: u64,
}

impl NoteCache {
    pub fn new() -> Self {
        // Below 2^53, so the page can hold it in a number.
        let start = u64::from(getrandom::u32().unwrap_or(0)) << 20;
        Self {
            open: HashMap::new(),
            version: start,
            uses: 0,
            previews: HashMap::new(),
            answered: VecDeque::new(),
            next_turn: Instant::now(),
        }
    }

    fn next_version(&mut self) -> u64 {
        self.version += 1;
        self.version
    }

    /// The open note, reading it from the log first if need be.
    fn open(&mut self, log: &OpLog, note: NoteRef) -> &mut LiveNote {
        self.uses += 1;
        let used = self.uses;
        if !self.open.contains_key(&note) {
            if self.open.len() >= MAX_OPEN {
                if let Some(oldest) = self.open.iter().min_by_key(|(_, o)| o.used).map(|(n, _)| *n) {
                    self.open.remove(&oldest);
                }
            }
            let version = self.next_version();
            let live = LiveNote::new(log.build_note(note), version);
            self.open.insert(note, OpenNote { live, used });
        }
        let open = self.open.get_mut(&note).expect("opened above");
        open.used = used;
        &mut open.live
    }

    fn answered(&self, editor: &EditorId) -> u64 {
        self.answered.iter().find(|(e, _)| e == editor).map_or(0, |(_, n)| *n)
    }

    fn answer(&mut self, editor: EditorId, n: u64) {
        self.answered.retain(|(e, _)| *e != editor);
        self.answered.push_back((editor, n));
        if self.answered.len() > MAX_EDITORS {
            self.answered.pop_front();
        }
    }

    /// Operations arrived (and, if `rebuild`, others may have been dropped):
    /// bring open notes up to date, and say what changed.
    fn absorb(&mut self, log: &OpLog, fresh: &[Op], rebuild: bool) -> Vec<Event> {
        let mut events = Vec::new();
        let mut list_changed = false;
        if rebuild {
            self.previews.clear();
            list_changed = true;
            self.open.retain(|note, _| log.note_exists(*note));
            let notes: BTreeSet<NoteRef> = self.open.keys().copied().collect();
            let mut docs = log.build_notes(&notes);
            for (note, open) in &mut self.open {
                self.version += 1;
                let patch = open.live.replace(docs.remove(note).unwrap_or_default(), self.version);
                events.extend(patch.map(|p| patch_event(*note, p, None)));
            }
        } else {
            let trust = log.trust();
            let effective = |op: &&Op| op.device == log.me() || trust.accepts(op.device, op.seq);
            let mut edits: BTreeMap<NoteRef, Vec<_>> = BTreeMap::new();
            for op in fresh.iter().filter(effective) {
                match &op.kind {
                    OpKind::NoteEdit { note, edit } => edits.entry(*note).or_default().push((op.device, edit)),
                    OpKind::NoteCreate { .. } | OpKind::NoteMeta { .. } => list_changed = true,
                    _ => {}
                }
            }
            for (note, edits) in edits {
                self.previews.remove(&note);
                let Some(open) = self.open.get_mut(&note) else {
                    list_changed = true;
                    continue;
                };
                self.version += 1;
                let patch = open.live.apply(edits, self.version);
                events.extend(patch.map(|p| patch_event(note, p, None)));
            }
        }
        // A note a peer has filled past what can be opened is closed here, and
        // opening it again says so.
        let oversized: Vec<NoteRef> =
            self.open.iter().filter(|(_, o)| o.live.doc().len_bytes() > MAX_OPEN_BYTES).map(|(n, _)| *n).collect();
        for note in oversized {
            self.open.remove(&note);
            events.retain(|e| !matches!(e, Event::NotePatch { note: n, .. } if *n == note));
            list_changed = true;
        }
        if list_changed {
            events.push(Event::NotesChanged);
        }
        events
    }
}

fn patch_event(note: NoteRef, p: Patch, by: Option<(EditorId, u64)>) -> Event {
    Event::NotePatch { note, from: p.from, to: p.to, change: p.change, by }
}

/// What became of an editor's edit.
#[derive(Debug, PartialEq, Eq)]
pub enum Edited {
    /// Recorded; these operations are to be broadcast (none if it changed nothing).
    Done(Vec<Op>),
    /// Made against a version this daemon does not remember.
    Stale,
    /// Looks like a secret, and the user has not said to keep it.
    Secret(&'static str),
}

fn note_error(e: EditError) -> anyhow::Error {
    match e {
        EditError::OutOfRange => anyhow!("that edit does not fit the note"),
        EditError::TooLarge => anyhow!("a note holds at most {} KiB of text", MAX_NOTE_BYTES >> 10),
    }
}

fn too_big_to_open(doc: &Doc) -> Result<()> {
    if doc.len_bytes() > MAX_OPEN_BYTES {
        bail!("this note is too large to open ({} MiB)", doc.len_bytes() >> 20);
    }
    Ok(())
}

/// A title or notebook as it is stored: clean, trimmed, bounded.
fn label(text: &str, max: usize) -> String {
    clean_text(text, max).trim().to_string()
}

/// The start of a note as one line, for the list.
fn preview(text: &str) -> String {
    let start: String = text.chars().take(4 * PREVIEW_CHARS).collect();
    let words: Vec<&str> = start.split_whitespace().collect();
    let line: String = words.join(" ").chars().take(PREVIEW_CHARS).collect();
    clean_text(&line, usize::MAX)
}

/// The kind of secret `delta` would put into a note, if any.
fn secret_added(doc: &Doc, delta: &Delta) -> Option<&'static str> {
    if delta.inserted_bytes() == 0 {
        return None;
    }
    let mut at = 0usize;
    for (k, step) in delta.steps().iter().enumerate() {
        match step {
            Step::Keep(n) | Step::Delete(n) => at += *n as usize,
            Step::Insert(inserted) => {
                // The words it lands in: what is kept on either side of it.
                let after = match delta.steps().get(k + 1) {
                    Some(Step::Delete(n)) => at + *n as usize,
                    _ => at,
                };
                let word = |c: &&char| !c.is_whitespace();
                let from = at.saturating_sub(SECRET_REACH);
                let near: Vec<char> = doc.text_between(from, after + SECRET_REACH).chars().collect();
                let (head, tail) = (near.get(..at - from)?, near.get(after - from..)?);
                let head = &head[head.len() - head.iter().rev().take_while(word).count()..];
                let head: String = head.iter().collect();
                let tail: String = tail.iter().take_while(word).collect();
                if let Some(kind) = content::secret_in_note(inserted, &format!("{head}{inserted}{tail}")) {
                    return Some(kind);
                }
            }
        }
    }
    None
}

/// Make `delta` in an open note, as one or more signed operations, and tell
/// subscribers. Returns the operations, oldest first.
fn make(ctx: &Ctx, s: &mut State, note: NoteRef, delta: Delta, by: Option<(EditorId, u64)>) -> Result<Vec<Op>> {
    let now = ctx.clock.now_ms();
    let me = s.log.me();
    let version = s.notes.next_version();
    let mut ops = Vec::new();
    let log = &mut s.log;
    let open = s.notes.open.get_mut(&note).ok_or_else(|| anyhow!("the note is not open"))?;
    let made = open.live.type_in(me, delta, version, |doc, delta| {
        let (edit, rest) = doc.edit_for(log.next_note_counter(), delta).map_err(note_error)?;
        let op = log
            .append_local(OpKind::NoteEdit { note, edit: edit.clone() }, now)
            .map_err(|e| anyhow!("could not record the edit: {e:?}"))?;
        ops.push(op);
        Ok::<_, anyhow::Error>((edit, rest))
    });
    if made.is_err() && !ops.is_empty() {
        // Nothing should fail after the first part has been recorded (the limits
        // are checked for the whole change first), but if something ever does,
        // the log decides: read the note again.
        s.notes.open.remove(&note);
    }
    Ctx::persist_all(s, &ops);
    let patch = made?;
    s.notes.previews.remove(&note);
    match (patch, by) {
        (Some(p), by) => ctx.emit(patch_event(note, p, by)),
        (None, Some(by)) => ctx.emit(Event::NoteNoop { note, by }),
        (None, None) => {}
    }
    Ok(ops)
}

impl Ctx {
    /// Take in what arrived for notes, telling subscribers before the lock is
    /// released, so that every subscriber sees a note's patches in order.
    pub(crate) fn absorb_notes(&self, s: &mut State, fresh: &[Op], rebuild: bool) {
        let events = s.notes.absorb(&s.log, fresh, rebuild);
        for event in events {
            self.emit(event);
        }
    }

    /// Every note, newest change first, at most [`MAX_NOTES`].
    pub fn notes(&self) -> Vec<NoteSummary> {
        let mut s = self.lock();
        let State { log, notes: cache, .. } = &mut *s;
        let mut all: Vec<_> = log.notes().into_iter().collect();
        all.sort_by_key(|(note, info)| std::cmp::Reverse((info.updated, *note)));
        all.truncate(MAX_NOTES);
        // Read the ones whose text changed since it was last previewed, in one
        // pass over the log. Notes with no text are remembered as such.
        let unread: BTreeSet<NoteRef> = all
            .iter()
            .map(|(note, _)| *note)
            .filter(|note| !cache.previews.contains_key(note) && !cache.open.contains_key(note))
            .collect();
        let docs = if unread.is_empty() { BTreeMap::new() } else { log.build_notes(&unread) };
        let seen = |doc: &Doc| (preview(&doc.text()), u32::try_from(doc.len_chars()).unwrap_or(u32::MAX));
        for note in unread {
            cache.previews.insert(note, docs.get(&note).map(seen).unwrap_or_default());
        }
        for (note, open) in &cache.open {
            cache.previews.entry(*note).or_insert_with(|| seen(open.live.doc()));
        }
        all.into_iter()
            .map(|(note, info)| {
                let (preview, chars) = cache.previews.get(&note).cloned().unwrap_or_default();
                NoteSummary {
                    note,
                    // Clean again: these may come from a peer.
                    title: label(&info.title, MAX_TITLE_BYTES),
                    notebook: label(&info.notebook, MAX_NOTEBOOK_BYTES),
                    pinned: info.pinned,
                    trashed: info.trashed,
                    created: info.created,
                    updated: info.updated,
                    updated_by: info.updated_by,
                    preview,
                    chars,
                }
            })
            .collect()
    }

    /// A new, empty note. Returns its name and the operations to broadcast.
    pub fn note_create(&self, title: &str, notebook: &str) -> Result<(NoteRef, Vec<Op>)> {
        let (title, notebook) = (label(title, MAX_TITLE_BYTES), label(notebook, MAX_NOTEBOOK_BYTES));
        let now = self.clock.now_ms();
        let mut s = self.lock();
        if s.log.is_revoked() {
            bail!("this device was removed from its group");
        }
        if s.log.notes().len() >= MAX_NOTES {
            bail!("there are already {MAX_NOTES} notes");
        }
        let op = s.log.append_local(OpKind::NoteCreate { title }, now).map_err(|e| anyhow!("{e:?}"))?;
        let note = NoteRef { device: op.device, seq: op.seq };
        let mut ops = vec![op];
        if !notebook.is_empty() {
            let kind = OpKind::NoteMeta { note, title: None, notebook: Some(notebook), pinned: None, trashed: None };
            ops.push(s.log.append_local(kind, now).map_err(|e| anyhow!("{e:?}"))?);
        }
        Self::persist_all(&mut s, &ops);
        self.emit(Event::NotesChanged);
        Ok((note, ops))
    }

    /// A note's text, kept open from now on. `editor` learns which of its
    /// edits the text includes.
    pub fn note_open(&self, note: NoteRef, editor: Option<EditorId>) -> Result<NoteText> {
        let mut s = self.lock();
        let State { log, notes: cache, .. } = &mut *s;
        if !log.note_exists(note) {
            bail!("that note does not exist (or was removed with the device that made it)");
        }
        let live = cache.open(log, note);
        if let Err(e) = too_big_to_open(live.doc()) {
            cache.open.remove(&note);
            return Err(e);
        }
        let (version, text, authors) = (live.version(), live.doc().text(), live.authors());
        let answered = editor.map_or(0, |e| cache.answered(&e));
        Ok(NoteText { version, text, authors, answered })
    }

    /// Wait for this edit's turn: signed edits are spaced out, whatever an
    /// editor sends. Not while holding the lock.
    pub async fn note_turn(&self) {
        let wait = {
            let mut s = self.lock();
            let now = Instant::now();
            let at = s.notes.next_turn.max(now);
            s.notes.next_turn = at + EDIT_GAP;
            at - now
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    /// Typing in an editor on this device.
    pub fn note_edit(&self, edit: NoteEdit) -> Result<Edited> {
        let NoteEdit { note, base, delta, editor, n, allow_secret } = edit;
        let mut s = self.lock();
        if s.log.is_revoked() {
            bail!("this device was removed from its group");
        }
        if !s.log.note_exists(note) {
            bail!("that note does not exist (or was removed with the device that made it)");
        }
        // A repeat of an edit already made (its reply was lost): it stands.
        if n <= s.notes.answered(&editor) {
            return Ok(Edited::Done(Vec::new()));
        }
        // Only an open note has versions; one that was closed to make room, or
        // a daemon that restarted, has to be opened again.
        let Some(delta) = s.notes.open.get(&note).and_then(|open| open.live.rebase(base, &delta)) else {
            return Ok(Edited::Stale);
        };
        let live = &s.notes.open[&note].live;
        too_big_to_open(live.doc())?;
        if !allow_secret {
            if let Some(kind) = secret_added(live.doc(), &delta) {
                return Ok(Edited::Secret(kind));
            }
        }
        s.notes.uses += 1;
        let used = s.notes.uses;
        s.notes.open.get_mut(&note).expect("checked above").used = used;
        if delta.is_empty() {
            s.notes.answer(editor, n);
            self.emit(Event::NoteNoop { note, by: (editor, n) });
            return Ok(Edited::Done(Vec::new()));
        }
        let ops = make(self, &mut s, note, delta, Some((editor, n)))?;
        s.notes.answer(editor, n);
        Ok(Edited::Done(ops))
    }

    /// Change a note's details on every device.
    pub fn note_meta(
        &self,
        note: NoteRef,
        title: Option<String>,
        notebook: Option<String>,
        pinned: Option<bool>,
        trashed: Option<bool>,
    ) -> Result<Vec<Op>> {
        if note == NoteRef::SCRATCHPAD {
            bail!("the scratchpad has no title or notebook, and cannot be pinned or thrown away");
        }
        if title.is_none() && notebook.is_none() && pinned.is_none() && trashed.is_none() {
            return Ok(Vec::new());
        }
        let title = title.map(|t| label(&t, MAX_TITLE_BYTES));
        let notebook = notebook.map(|n| label(&n, MAX_NOTEBOOK_BYTES));
        let now = self.clock.now_ms();
        let mut s = self.lock();
        if s.log.is_revoked() {
            bail!("this device was removed from its group");
        }
        if !s.log.note_exists(note) {
            bail!("that note does not exist (or was removed with the device that made it)");
        }
        let kind = OpKind::NoteMeta { note, title, notebook, pinned, trashed };
        let ops = vec![s.log.append_local(kind, now).map_err(|e| anyhow!("{e:?}"))?];
        Self::persist_all(&mut s, &ops);
        self.emit(Event::NotesChanged);
        Ok(ops)
    }

    pub fn note_history(&self, note: NoteRef) -> Result<Vec<NoteRevision>> {
        let s = self.lock();
        if !s.log.note_exists(note) {
            bail!("that note does not exist (or was removed with the device that made it)");
        }
        Ok(s.log.note_history(note))
    }

    /// The note as it was at a revision.
    pub fn note_at(&self, note: NoteRef, at: Timestamp, author: DeviceId) -> Result<NoteText> {
        let s = self.lock();
        let doc = s.log.note_exists(note).then(|| s.log.note_at(note, at, author)).flatten();
        let doc = doc.ok_or_else(|| anyhow!("that version does not exist"))?;
        too_big_to_open(&doc)?;
        Ok(NoteText { version: 0, text: doc.text(), authors: doc.authors(), answered: 0 })
    }

    /// Make the note's text what it was at a revision, as a new edit: nothing
    /// is lost, and restoring can itself be undone from the history.
    pub fn note_restore(&self, note: NoteRef, at: Timestamp, author: DeviceId) -> Result<Vec<Op>> {
        let mut s = self.lock();
        if s.log.is_revoked() {
            bail!("this device was removed from its group");
        }
        let old = s.log.note_exists(note).then(|| s.log.note_at(note, at, author)).flatten();
        let old = old.ok_or_else(|| anyhow!("that version does not exist"))?;
        let State { log, notes: cache, .. } = &mut *s;
        let live = cache.open(log, note);
        too_big_to_open(live.doc())?;
        // (Compared as text: characters a restore put back are new characters.)
        if live.doc().text() == old.text() {
            return Ok(Vec::new());
        }
        let delta = live.doc().delta_to(&old);
        make(self, &mut s, note, delta, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ctx::tests::{ctx, peer_of};
    use tether_core::delta::transform;

    /// An editor window, as the page implements it: what it was sent, what it
    /// shows, and what it has typed that has not been answered yet.
    struct Editor {
        id: EditorId,
        note: NoteRef,
        version: u64,
        text: String,
        n: u64,
        /// Sent, not answered.
        sent: Option<Delta>,
        base_text: String,
    }

    impl Editor {
        fn open(ctx: &Ctx, note: NoteRef, id: u8) -> Self {
            let opened = ctx.note_open(note, Some([id; 16])).unwrap();
            let (version, text) = (opened.version, opened.text);
            Self { id: [id; 16], note, version, base_text: text.clone(), text, n: 0, sent: None }
        }

        fn typed(&mut self, delta: Delta) -> NoteEdit {
            assert!(self.sent.is_none(), "one at a time");
            self.text = delta.apply(&self.text).unwrap();
            self.sent = Some(delta.clone());
            self.n += 1;
            let Editor { id, note, version, n, .. } = *self;
            NoteEdit { note, base: version, delta, editor: id, n, allow_secret: false }
        }

        fn sees(&mut self, event: &Event) {
            match event {
                Event::NotePatch { note, from, to, change, by } if *note == self.note => {
                    if *to <= self.version {
                        return; // from before it opened the note
                    }
                    assert_eq!(*from, self.version, "patches follow one another");
                    self.base_text = change.delta.apply(&self.base_text).unwrap();
                    self.version = *to;
                    if *by == Some((self.id, self.n)) {
                        self.sent = None;
                        assert_eq!(self.text, self.base_text);
                    } else if let Some(sent) = self.sent.take() {
                        let (sent, theirs) = transform(&sent, &change.delta);
                        self.text = theirs.apply(&self.text).unwrap();
                        self.sent = Some(sent);
                    } else {
                        self.text = self.base_text.clone();
                    }
                }
                Event::NoteNoop { by, .. } if *by == (self.id, self.n) => self.sent = None,
                _ => {}
            }
        }
    }

    fn done(e: Edited) -> Vec<Op> {
        match e {
            Edited::Done(ops) => ops,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn typing_is_recorded_shown_and_survives_a_rebuild() {
        let (ctx, _) = ctx();
        let (note, ops) = ctx.note_create(" Ideas \u{202E}", "Work").unwrap();
        assert_eq!(ops.len(), 2);
        let mut events = ctx.events.subscribe();
        let mut e = Editor::open(&ctx, note, 1);
        for (at, word) in [(0, "hello"), (5, " world"), (0, "> ")] {
            let ops = done(ctx.note_edit(e.typed(Delta::splice(at, 0, word))).unwrap());
            assert_eq!(ops.len(), 1, "one operation per batch");
            e.sees(&events.try_recv().unwrap());
        }
        assert_eq!(e.text, "> hello world");
        assert_eq!(ctx.lock().log.note_doc(note).unwrap().text(), e.text);
        let list = ctx.notes();
        let summary = list.iter().find(|n| n.note == note).unwrap();
        assert_eq!((summary.title.as_str(), summary.notebook.as_str()), ("Ideas", "Work"));
        assert_eq!((summary.preview.as_str(), summary.chars), ("> hello world", 13));
        assert!(list.iter().any(|n| n.note == NoteRef::SCRATCHPAD), "the scratchpad is always there");
    }

    #[test]
    fn a_repeated_edit_is_made_once() {
        let (ctx, _) = ctx();
        let mut e = Editor::open(&ctx, NoteRef::SCRATCHPAD, 1);
        let edit = e.typed(Delta::splice(0, 0, "once"));
        assert_eq!(done(ctx.note_edit(edit.clone()).unwrap()).len(), 1);
        assert_eq!(done(ctx.note_edit(edit).unwrap()).len(), 0, "the reply was lost and the app asked again");
        assert_eq!(ctx.lock().log.note_doc(NoteRef::SCRATCHPAD).unwrap().text(), "once");
        assert_eq!(ctx.note_open(NoteRef::SCRATCHPAD, Some(e.id)).unwrap().answered, 1);
        assert_eq!(ctx.note_open(NoteRef::SCRATCHPAD, Some([9; 16])).unwrap().answered, 0);
    }

    #[test]
    fn two_windows_and_another_device_stay_in_step() {
        let (ctx, _) = ctx();
        let mut peer = peer_of(&ctx, 2);
        let note = NoteRef::SCRATCHPAD;
        let mut events = ctx.events.subscribe();
        let (mut a, mut b) = (Editor::open(&ctx, note, 1), Editor::open(&ctx, note, 2));
        let mut seed = 7u64;
        let mut roll = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for round in 0..400 {
            // Each window may type (against whatever it has seen) before either
            // hears about anything else that happened this round.
            for e in [&mut a, &mut b] {
                if e.sent.is_none() && roll(3) > 0 {
                    let len = e.text.chars().count() as u64;
                    let at = roll(len + 1);
                    let delta =
                        Delta::splice(at as usize, roll((len - at).min(3) + 1) as usize, &"abé😀"[..roll(3) as usize]);
                    if !delta.is_empty() {
                        done(ctx.note_edit(e.typed(delta)).unwrap());
                    }
                }
            }
            if roll(2) == 0 {
                // The peer types too, from its own copy of the note.
                let (missing, _) = ctx.lock().log.missing_for(&peer.version_vector());
                if roll(3) == 0 {
                    peer.apply_batch(missing, 1);
                }
                let doc = peer.build_note(note);
                let at = roll(doc.len_chars() as u64 + 1) as usize;
                let edit = doc.local_edit(peer.next_note_counter(), at, 0, "peer").unwrap();
                let op = peer.append_local(OpKind::NoteEdit { note, edit }, 1_000 + round).unwrap();
                ctx.apply(vec![op]);
            }
            while let Ok(event) = events.try_recv() {
                a.sees(&event);
                b.sees(&event);
            }
        }
        let text = ctx.lock().log.note_doc(note).unwrap().text();
        assert!(text.len() > 200);
        assert_eq!(a.text, text);
        assert_eq!(b.text, text);
        assert_eq!(ctx.note_open(note, None).unwrap().text, text);
    }

    #[test]
    fn a_big_paste_is_one_patch_and_several_operations() {
        let (ctx, _) = ctx();
        let mut events = ctx.events.subscribe();
        let mut e = Editor::open(&ctx, NoteRef::SCRATCHPAD, 1);
        let text = "a line of ordinary text\n".repeat(1_000);
        let ops = done(ctx.note_edit(e.typed(Delta::splice(0, 0, &text))).unwrap());
        assert!(ops.len() > 1);
        e.sees(&events.try_recv().unwrap());
        assert!(e.sent.is_none() && events.try_recv().is_err(), "one patch answers it");
        assert_eq!(ctx.lock().log.note_doc(NoteRef::SCRATCHPAD).unwrap().text(), text);
        let over = Delta::splice(0, 0, &"x".repeat(MAX_NOTE_BYTES));
        let refused = ctx.note_edit(e.typed(over)).unwrap_err();
        assert!(refused.to_string().contains("256 KiB"), "{refused}");
        assert_eq!(ctx.lock().log.note_doc(NoteRef::SCRATCHPAD).unwrap().text(), text, "nothing was recorded");
        // A refusal leaves the note open: the editor carries on where it was.
        assert_eq!(ctx.lock().notes.open[&NoteRef::SCRATCHPAD].live.version(), e.version);
    }

    #[test]
    fn secrets_wait_for_the_user() {
        let (ctx, _) = ctx();
        let mut e = Editor::open(&ctx, NoteRef::SCRATCHPAD, 1);
        let before = ctx.lock().log.len();
        let mut edit = e.typed(Delta::splice(0, 0, "aws: AKIAIOSFODNN7EXAMPLE"));
        assert_eq!(ctx.note_edit(edit.clone()).unwrap(), Edited::Secret("AWS access key"));
        assert_eq!(ctx.lock().log.len(), before, "nothing was recorded");
        edit.allow_secret = true;
        assert_eq!(done(ctx.note_edit(edit).unwrap()).len(), 1, "the user said to keep it");
        // Completing one key by key counts too.
        let mut e = Editor::open(&ctx, NoteRef::SCRATCHPAD, 2);
        let end = e.text.chars().count();
        let token = format!("\nghp_{}", "a".repeat(35));
        done(ctx.note_edit(NoteEdit { allow_secret: true, ..e.typed(Delta::splice(end, 0, &token)) }).unwrap());
        let mut e = Editor::open(&ctx, NoteRef::SCRATCHPAD, 3);
        let end = e.text.chars().count();
        assert_eq!(ctx.note_edit(e.typed(Delta::splice(end, 0, "a"))).unwrap(), Edited::Secret("GitHub token"));
    }

    #[test]
    fn versions_from_another_run_or_a_closed_note_are_stale() {
        let (ctx, _) = ctx();
        let mut e = Editor::open(&ctx, NoteRef::SCRATCHPAD, 1);
        e.version = 12; // from before a restart
        assert_eq!(ctx.note_edit(e.typed(Delta::splice(0, 0, "x"))).unwrap(), Edited::Stale);

        let mut e = Editor::open(&ctx, NoteRef::SCRATCHPAD, 2);
        for k in 0..MAX_OPEN {
            let (note, _) = ctx.note_create(&format!("{k}"), "").unwrap();
            ctx.note_open(note, None).unwrap();
        }
        assert_eq!(ctx.lock().notes.open.len(), MAX_OPEN);
        assert_eq!(ctx.note_edit(e.typed(Delta::splice(0, 0, "x"))).unwrap(), Edited::Stale);
    }

    #[test]
    fn bad_edits_and_missing_notes_are_refused() {
        let (ctx, _) = ctx();
        let mut e = Editor::open(&ctx, NoteRef::SCRATCHPAD, 1);
        assert!(ctx.note_edit(e.typed_unchecked(Delta::splice(5, 0, "x"))).is_err(), "past the end");
        let missing = NoteRef { device: ctx.me(), seq: 77 };
        assert!(ctx.note_open(missing, None).is_err());
        assert!(ctx.note_history(missing).is_err());
        assert!(ctx.note_meta(missing, None, None, Some(true), None).is_err());
        assert!(ctx.note_meta(NoteRef::SCRATCHPAD, Some("x".into()), None, None, None).is_err());
        assert!(ctx.note_restore(NoteRef::SCRATCHPAD, Timestamp::default(), ctx.me()).is_err());
        assert_eq!(ctx.lock().log.len(), 0, "nothing was recorded");
    }

    impl Editor {
        fn typed_unchecked(&mut self, delta: Delta) -> NoteEdit {
            self.n += 1;
            let Editor { id, note, version, n, .. } = *self;
            NoteEdit { note, base: version, delta, editor: id, n, allow_secret: false }
        }
    }

    #[test]
    fn restoring_a_version_is_an_edit_like_any_other() {
        let (ctx, _) = ctx();
        let (note, _) = ctx.note_create("n", "").unwrap();
        let mut events = ctx.events.subscribe();
        let mut e = Editor::open(&ctx, note, 1);
        done(ctx.note_edit(e.typed(Delta::splice(0, 0, "first version"))).unwrap());
        e.sees(&events.try_recv().unwrap());
        let first = ctx.note_history(note).unwrap()[0];
        done(ctx.note_edit(e.typed(Delta::splice(0, 5, "second"))).unwrap());
        e.sees(&events.try_recv().unwrap());
        assert_eq!(e.text, "second version");

        assert_eq!(ctx.note_at(note, first.at, first.author).unwrap().text, "first version");
        assert_eq!(ctx.note_restore(note, first.at, first.author).unwrap().len(), 1);
        e.sees(&events.try_recv().unwrap());
        assert_eq!(e.text, "first version");
        assert!(ctx.note_restore(note, first.at, first.author).unwrap().is_empty(), "already so");
    }

    #[test]
    fn revoking_a_device_takes_back_what_it_wrote_after() {
        let (ctx, _) = ctx();
        let mut peer = peer_of(&ctx, 2);
        let note = NoteRef::SCRATCHPAD;
        let mut events = ctx.events.subscribe();
        let mut e = Editor::open(&ctx, note, 1);
        let type_as = |peer: &mut OpLog, at: usize, text: &str| {
            let edit = peer.build_note(note).local_edit(peer.next_note_counter(), at, 0, text).unwrap();
            peer.append_local(OpKind::NoteEdit { note, edit }, 5_000).unwrap()
        };
        let before = type_as(&mut peer, 0, "before ");
        ctx.apply(vec![before]);
        // A third member revoked the peer here, before this later edit reaches us.
        let third_key = tether_core::DeviceKey::for_test(3);
        let now = ctx.clock.now_ms();
        let add = OpKind::DeviceAdded { device: third_key.id(), name: "third".into() };
        ctx.lock().log.append_local(add, now).unwrap();
        let mut third = OpLog::new(third_key);
        third.set_root(ctx.me());
        let (all, _) = ctx.lock().log.missing_for(&Default::default());
        third.apply_batch(all, now);
        let revoke = OpKind::DeviceRevoked { device: peer.me(), after_seq: 1 };
        let revoke = third.append_local(revoke, now + 1).unwrap();
        let after = type_as(&mut peer, 7, "after");
        ctx.apply(vec![after]);
        assert_eq!(ctx.note_open(note, None).unwrap().text, "before after");
        ctx.apply(vec![revoke]);
        assert_eq!(ctx.note_open(note, None).unwrap().text, "before ");
        while let Ok(event) = events.try_recv() {
            e.sees(&event);
        }
        assert_eq!(e.text, "before ", "an open editor sees it go");
    }

    #[test]
    fn a_note_whose_maker_is_gone_closes() {
        let (ctx, _) = ctx();
        let mut peer = peer_of(&ctx, 2);
        let create = peer.append_local(OpKind::NoteCreate { title: "theirs".into() }, 5_000).unwrap();
        let note = NoteRef { device: create.device, seq: create.seq };
        ctx.apply(vec![create]);
        ctx.note_open(note, None).unwrap();
        // Another member's revocation, from before the note was made.
        let now = ctx.clock.now_ms();
        ctx.lock().log.append_local(OpKind::DeviceRevoked { device: peer.me(), after_seq: 0 }, now).unwrap();
        let mut s = ctx.lock();
        ctx.absorb_notes(&mut s, &[], true);
        drop(s);
        assert!(ctx.note_open(note, None).is_err());
        assert!(ctx.lock().notes.open.is_empty());
    }

    /// Timings for the biggest note allowed, by hand:
    /// `cargo test --release -p tether-daemon note_timings -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn note_timings() {
        let (ctx, _) = ctx();
        let mut events = ctx.events.subscribe();
        for kib in [10usize, 100, 250] {
            let (note, _) = ctx.note_create("big", "").unwrap();
            let mut e = Editor::open(&ctx, note, (kib + 1) as u8);
            let text = "The quick brown fox jumps over the lazy dog. ".repeat(kib * 1024 / 45);
            let started = Instant::now();
            done(ctx.note_edit(e.typed(Delta::splice(0, 0, &text))).unwrap());
            println!("{kib} KiB: paste {:.1?}", started.elapsed());
            while events.try_recv().is_ok() {}
            let mut e = Editor::open(&ctx, note, kib as u8);
            let mut took: Vec<Duration> = (0..200)
                .map(|k| {
                    let edit = e.typed(Delta::splice(kib * 512 + k, 0, "k"));
                    let started = Instant::now();
                    done(ctx.note_edit(edit).unwrap());
                    let took = started.elapsed();
                    e.sees(&events.try_recv().unwrap());
                    took
                })
                .collect();
            took.sort();
            println!("{kib} KiB: keystroke p50 {:.2?} p99 {:.2?}", took[100], took[197]);
            let mut s = ctx.lock();
            s.notes.open.clear();
            drop(s);
            let started = Instant::now();
            ctx.note_open(note, None).unwrap();
            println!("{kib} KiB: open from the log {:.1?}", started.elapsed());
        }
    }
}
