//! A note held open in memory, for editors on this device.
//!
//! Edits from every device are applied as they arrive, and each change to the
//! visible text becomes a [`Patch`] from one version to the next. Editors
//! follow the patches. An editor's own change is made against the version it
//! last saw, so it is first carried over the patches since ([`LiveNote::rebase`]);
//! a version too old for that (or from before a restart) is refused, and the
//! editor fetches the text again.
//!
//! Pure, like the rest of this crate: the simulator runs it with edits arriving
//! in every order the network can produce and editors that lag behind, and
//! checks that the note equals the log and that each editor equals the note.

use std::collections::VecDeque;

use crate::delta::{transform, Delta};
use crate::notes::{Doc, Edit, Span, TextChange, Touched};
use crate::DeviceId;

/// Patches kept for editors that lag behind.
pub const MAX_RECENT: usize = 512;
/// Most inserted text kept in them.
pub const MAX_RECENT_BYTES: usize = 1 << 20;

/// A change to a note's visible text, from one version to the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub from: u64,
    pub to: u64,
    pub change: TextChange,
}

#[derive(Debug, Clone)]
pub struct LiveNote {
    doc: Doc,
    version: u64,
    /// The latest patches, oldest first; each starts where the one before ended.
    recent: VecDeque<(u64, Delta)>,
    recent_bytes: usize,
}

impl LiveNote {
    /// Versions only ever compare equal or not; the caller makes sure one is
    /// never used twice (a counter that starts somewhere random).
    pub fn new(doc: Doc, version: u64) -> Self {
        Self { doc, version, recent: VecDeque::new(), recent_bytes: 0 }
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    /// `delta`, made by an editor that had seen version `base`, as a change to
    /// the text as it is now. `None` if `base` is not a version this note
    /// remembers.
    pub fn rebase(&self, base: u64, delta: &Delta) -> Option<Delta> {
        if base == self.version {
            return Some(delta.clone());
        }
        let start = self.recent.iter().position(|(from, _)| *from == base)?;
        Some(self.recent.range(start..).fold(delta.clone(), |d, (_, patch)| transform(&d, patch).0))
    }

    /// Apply edits, in any order, giving the text version `next` if it
    /// changed. Edits already applied, or still waiting for characters that
    /// have not arrived, change nothing.
    pub fn apply<'a>(&mut self, edits: impl IntoIterator<Item = (DeviceId, &'a Edit)>, next: u64) -> Option<Patch> {
        let mut touched = Touched::default();
        for (author, edit) in edits {
            self.doc.apply_tracked(author, edit, &mut touched);
        }
        let change = self.doc.change(&touched);
        self.advance(change, next)
    }

    /// Make a change typed on this device, in as many edits as it takes, as
    /// one new version. `record` is given the note and what is left to do, and
    /// signs and stores one edit ([`Doc::edit_for`]); if it fails, the note
    /// must be read from the log again.
    pub fn type_in<E>(
        &mut self,
        me: DeviceId,
        delta: Delta,
        next: u64,
        mut record: impl FnMut(&Doc, &Delta) -> Result<(Edit, Option<Delta>), E>,
    ) -> Result<Option<Patch>, E> {
        let mut touched = Touched::default();
        let mut rest = Some(delta);
        while let Some(todo) = rest {
            let (edit, more) = record(&self.doc, &todo)?;
            self.doc.apply_tracked(me, &edit, &mut touched);
            rest = more;
        }
        let change = self.doc.change(&touched);
        Ok(self.advance(change, next))
    }

    /// Replace the note with one rebuilt from the log (operations may have
    /// been dropped when a device was revoked).
    pub fn replace(&mut self, doc: Doc, next: u64) -> Option<Patch> {
        let mut change = doc.change_from(&self.doc);
        // `change_from` relies on both copies agreeing about order; if they
        // ever did not, show the rebuilt text rather than a wrong one.
        if change.delta.apply(&self.doc.text()).as_deref() != Some(&doc.text()) {
            change = TextChange { delta: Delta::splice(0, self.doc.len_chars(), &doc.text()), authors: doc.authors() };
        }
        self.doc = doc;
        self.advance(change, next)
    }

    fn advance(&mut self, change: TextChange, next: u64) -> Option<Patch> {
        if change.delta.is_empty() {
            return None;
        }
        let from = self.version;
        self.version = next;
        self.recent_bytes += change.delta.inserted_bytes();
        self.recent.push_back((from, change.delta.clone()));
        while self.recent.len() > MAX_RECENT || self.recent_bytes > MAX_RECENT_BYTES {
            let Some((_, dropped)) = self.recent.pop_front() else { break };
            self.recent_bytes -= dropped.inserted_bytes();
        }
        Some(Patch { from, to: next, change })
    }

    /// Who wrote the text, for an editor opening the note.
    pub fn authors(&self) -> Vec<Span> {
        self.doc.authors()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::first_counter;
    use crate::DeviceKey;

    #[test]
    fn a_stale_edit_is_carried_over_what_it_missed() {
        let (me, other) = (DeviceKey::for_test(1).id(), DeviceKey::for_test(2).id());
        let mut note = LiveNote::new(Doc::new(), 10);
        let edit = note.doc().local_edit(first_counter(1).unwrap(), 0, 0, "hello").unwrap();
        let patch = note.apply([(me, &edit)], 11).unwrap();
        assert_eq!((patch.from, patch.to), (10, 11));
        assert_eq!(patch.change.authors, [Span { device: me, chars: 5 }]);
        assert!(note.apply([(me, &edit)], 12).is_none(), "applied twice changes nothing");
        assert_eq!(note.version(), 11);

        let edit = note.doc().local_edit(first_counter(1).unwrap(), 0, 0, ">> ").unwrap();
        note.apply([(other, &edit)], 12).unwrap();
        // An editor still at version 11 adds "!" at the end of "hello".
        let d = note.rebase(11, &Delta::splice(5, 0, "!")).unwrap();
        assert_eq!(d, Delta::splice(8, 0, "!"));
        assert_eq!(note.rebase(7, &Delta::splice(0, 0, "x")), None, "a version it never had");
    }

    #[test]
    fn old_patches_are_forgotten_and_stay_bounded() {
        let me = DeviceKey::for_test(1).id();
        let mut note = LiveNote::new(Doc::new(), 1);
        for k in 1..=(MAX_RECENT as u64 + 5) {
            let edit = note.doc().local_edit(first_counter(k).unwrap(), 0, 0, "x").unwrap();
            note.apply([(me, &edit)], k + 1).unwrap();
        }
        assert_eq!(note.recent.len(), MAX_RECENT);
        assert!(note.rebase(1, &Delta::new()).is_none());
        assert!(note.rebase(note.version() - 10, &Delta::new()).is_some());
    }
}
