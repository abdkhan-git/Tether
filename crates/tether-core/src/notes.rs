//! Notes (F11): a sequence CRDT for text, so a note can be edited on several
//! devices at once, or apart for hours, and merge without losing or scrambling
//! either side's edits. Design: docs/notes-design.md.
//!
//! This is **Fugue** in its tree form (Weidner and Kleppmann, "The Art of the
//! Fugue", 2023). Every character is a node with a permanent id, a parent and a
//! side; the text is an in-order walk of the tree (left children, the node,
//! right children), with siblings ordered by id. Deleted characters stay as
//! tombstones. The one insertion rule (see [`Doc::local_edit`]) is what keeps
//! text typed concurrently at the same place from interleaving, forwards or
//! backwards.
//!
//! The document depends only on the set of edits applied, never on their
//! order: a run whose parent has not arrived waits, and a delete of a
//! character not yet seen is remembered. Pure and deterministic, so the daemon
//! and the simulator run the same code.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::delta::{Delta, Step};
use crate::DeviceId;

/// Most text one edit may insert.
pub const MAX_EDIT_BYTES: usize = 4 << 10;
/// Most runs, and most delete ranges, in one edit.
pub const MAX_EDIT_PARTS: usize = 1024;
/// Most characters one edit may delete (the sum of its ranges).
pub const MAX_EDIT_DELETES: u64 = 1 << 16;
/// Largest visible text a note may hold.
pub const MAX_NOTE_BYTES: usize = 256 << 10;
/// Longest note title.
pub const MAX_TITLE_BYTES: usize = 200;
/// Longest notebook name.
pub const MAX_NOTEBOOK_BYTES: usize = 64;
/// Most notes a device will create.
pub const MAX_NOTES: usize = 4096;

/// A character's permanent name: its author's per-device counter, then the
/// author. Ordered by counter first, so every device sorts siblings alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CharId {
    pub counter: u64,
    pub device: DeviceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}

/// Characters typed in a row. The first attaches to `(parent, side)`
/// (`parent: None` is the start of the note); each next one is the previous
/// one's right child. Ids are `first, first + 1, ...` by the edit's author.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub first: u64,
    pub parent: Option<CharId>,
    pub side: Side,
    pub text: String,
}

/// Deleted characters `device`'s `from..=to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdRange {
    pub device: DeviceId,
    pub from: u64,
    pub to: u64,
}

/// One batch of edits to a note, signed as one operation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    pub inserts: Vec<Run>,
    pub deletes: Vec<IdRange>,
}

impl Edit {
    pub fn is_empty(&self) -> bool {
        self.inserts.iter().all(|r| r.text.is_empty()) && self.deletes.is_empty()
    }

    /// Within the limits above, with sane ranges and counters.
    pub fn within_limits(&self) -> bool {
        let text: usize = self.inserts.iter().map(|r| r.text.len()).sum();
        let mut deleted = 0u64;
        for r in &self.deletes {
            if r.to < r.from {
                return false;
            }
            // `to - from + 1` overflows for a range ending at u64::MAX (and wraps to
            // 0 in release builds), which would let a huge range pass.
            deleted = deleted.saturating_add((r.to - r.from).saturating_add(1));
        }
        let counters_fit = self.inserts.iter().all(|r| r.first.checked_add(r.text.chars().count() as u64).is_some());
        text <= MAX_EDIT_BYTES
            && self.inserts.len() <= MAX_EDIT_PARTS
            && self.deletes.len() <= MAX_EDIT_PARTS
            && deleted <= MAX_EDIT_DELETES
            && counters_fit
    }

    /// Characters this edit inserts (the author's counter advances by this).
    pub fn inserted_chars(&self) -> u64 {
        self.inserts.iter().map(|r| r.text.chars().count() as u64).sum()
    }

    /// Do this edit's character ids belong to the operation `seq` that carries
    /// it, without overlapping? Character counters are `seq × 2^20 + offset`,
    /// so ids are unique by construction: an author cannot reuse one (which
    /// would let devices that saw two uses in different orders disagree), and
    /// any device can check this from the operation alone.
    pub fn ids_belong_to(&self, seq: u64) -> bool {
        let Some(base) = first_counter(seq) else { return false };
        let mut spans: Vec<(u64, u64)> = Vec::with_capacity(self.inserts.len());
        for r in &self.inserts {
            let len = r.text.chars().count() as u64;
            let Some(offset) = r.first.checked_sub(base) else { return false };
            if offset.saturating_add(len) > OFFSETS_PER_OP {
                return false;
            }
            spans.push((offset, offset + len));
        }
        spans.sort_unstable();
        spans.windows(2).all(|w| w[0].1 <= w[1].0)
    }
}

/// Characters one operation may insert: its ids are `seq × OFFSETS_PER_OP + offset`.
pub const OFFSETS_PER_OP: u64 = 1 << 20;

/// The first character counter of operation `seq`.
pub fn first_counter(seq: u64) -> Option<u64> {
    seq.checked_mul(OFFSETS_PER_OP)
}

/// So many characters in a row typed on one device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub device: DeviceId,
    pub chars: u32,
}

/// Add `n` characters by `device` to a list of spans.
fn push_span(spans: &mut Vec<Span>, device: DeviceId, n: u32) {
    match spans.last_mut() {
        Some(s) if s.device == device => s.chars = s.chars.saturating_add(n),
        _ => spans.push(Span { device, chars: n }),
    }
}

/// A change to a note's visible text, and who wrote the characters it inserts
/// (in order, covering all of its insertions).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextChange {
    pub delta: Delta,
    pub authors: Vec<Span>,
}

/// The characters a set of edits inserted into or removed from view, gathered
/// while applying them, so that the change can be read off in one walk.
#[derive(Debug, Default)]
pub struct Touched {
    inserted: HashSet<usize>,
    deleted: HashSet<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// The position or length is outside the note.
    OutOfRange,
    /// The edit, or the note after it, would exceed a limit.
    TooLarge,
}

const ROOT: usize = 0;

/// A node's children on one side, in order. Almost every character has none
/// or one, which are kept in place: a note of 100,000 characters is walked on
/// every edit, and a heap block per character made that several times slower.
#[derive(Clone, Debug, Default)]
enum Children {
    #[default]
    None,
    One(u32),
    Many(Vec<u32>),
}

impl Children {
    fn as_slice(&self) -> &[u32] {
        match self {
            Children::None => &[],
            Children::One(c) => std::slice::from_ref(c),
            Children::Many(c) => c,
        }
    }

    fn insert(&mut self, at: usize, child: u32) {
        match self {
            Children::None => *self = Children::One(child),
            Children::One(c) if at == 0 => *self = Children::Many(vec![child, *c]),
            Children::One(c) => *self = Children::Many(vec![*c, child]),
            Children::Many(c) => c.insert(at, child),
        }
    }
}

#[derive(Clone, Debug)]
struct Node {
    id: CharId,
    ch: char,
    deleted: bool,
    left: Children,
    right: Children,
}

/// One note's text, as a Fugue tree.
#[derive(Clone, Debug)]
pub struct Doc {
    /// `nodes[ROOT]` is the start of the note (no character).
    nodes: Vec<Node>,
    index: HashMap<CharId, usize>,
    /// Runs whose parent has not arrived, by that parent.
    waiting: HashMap<CharId, Vec<(DeviceId, Run)>>,
    /// Deletes of characters not seen yet.
    early_deletes: HashSet<CharId>,
    visible_bytes: usize,
    visible_chars: usize,
}

impl Default for Doc {
    fn default() -> Self {
        let root = Node {
            id: CharId { counter: 0, device: DeviceId([0; 32]) },
            ch: '\0',
            deleted: true,
            left: Children::None,
            right: Children::None,
        };
        Self {
            nodes: vec![root],
            index: HashMap::new(),
            waiting: HashMap::new(),
            early_deletes: HashSet::new(),
            visible_bytes: 0,
            visible_chars: 0,
        }
    }
}

impl Doc {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply an edit authored by `author`. Idempotent: characters already
    /// present are skipped. Assumes the edit passed [`Edit::within_limits`].
    pub fn apply(&mut self, author: DeviceId, edit: &Edit) {
        self.apply_tracked(author, edit, &mut Touched::default());
    }

    /// As [`Doc::apply`], noting in `touched` what came into or left view
    /// (read it with [`Doc::change`]).
    pub fn apply_tracked(&mut self, author: DeviceId, edit: &Edit, touched: &mut Touched) {
        let mut queue: VecDeque<(DeviceId, Run)> = edit.inserts.iter().map(|r| (author, r.clone())).collect();
        while let Some((author, run)) = queue.pop_front() {
            self.insert_run(author, run, &mut queue, touched);
        }
        for range in &edit.deletes {
            for counter in range.from..=range.to {
                let id = CharId { counter, device: range.device };
                match self.index.get(&id) {
                    Some(&i) => {
                        if self.mark_deleted(i) {
                            touched.deleted.insert(i);
                        }
                    }
                    None => {
                        self.early_deletes.insert(id);
                    }
                }
            }
        }
    }

    /// Hide a character; false if it already was.
    fn mark_deleted(&mut self, i: usize) -> bool {
        let node = &mut self.nodes[i];
        let shown = !node.deleted;
        if shown {
            node.deleted = true;
            self.visible_bytes -= node.ch.len_utf8();
            self.visible_chars -= 1;
        }
        shown
    }

    fn insert_run(&mut self, author: DeviceId, run: Run, queue: &mut VecDeque<(DeviceId, Run)>, touched: &mut Touched) {
        if run.text.is_empty() {
            return;
        }
        // The start of the note only ever has right children (see local_edit).
        let (parent, side) = match run.parent {
            None => (ROOT, Side::Right),
            Some(p) => match self.index.get(&p) {
                Some(&i) => (i, run.side),
                None => {
                    self.waiting.entry(p).or_default().push((author, run));
                    return;
                }
            },
        };
        let mut attach = (parent, side);
        for (k, ch) in run.text.chars().enumerate() {
            let Some(counter) = run.first.checked_add(k as u64) else { return };
            let id = CharId { counter, device: author };
            if let Some(&existing) = self.index.get(&id) {
                attach = (existing, Side::Right);
                continue;
            }
            let deleted = self.early_deletes.remove(&id);
            let i = self.nodes.len();
            self.nodes.push(Node { id, ch, deleted, left: Children::None, right: Children::None });
            self.index.insert(id, i);
            if !deleted {
                self.visible_bytes += ch.len_utf8();
                self.visible_chars += 1;
                touched.inserted.insert(i);
            }
            let siblings = match attach.1 {
                Side::Left => self.nodes[attach.0].left.as_slice(),
                Side::Right => self.nodes[attach.0].right.as_slice(),
            };
            let at = siblings.partition_point(|&s| self.nodes[s as usize].id < id);
            // A note is far smaller than 2^32 characters: each took an operation's bytes to make.
            let child = i as u32;
            match attach.1 {
                Side::Left => self.nodes[attach.0].left.insert(at, child),
                Side::Right => self.nodes[attach.0].right.insert(at, child),
            }
            attach = (i, Side::Right);
            if let Some(runs) = self.waiting.remove(&id) {
                queue.extend(runs);
            }
        }
    }

    /// Every node in text order (tombstones included), the root excluded,
    /// until `visit` returns false. Iterative: a long run is a long chain, too
    /// deep to recurse. Nothing is collected: this runs on every edit.
    fn walk(&self, mut visit: impl FnMut(usize) -> bool) {
        enum Frame {
            /// A subtree not yet started.
            Enter(u32),
            /// A node whose left children are done.
            Visit(u32),
        }
        let mut stack = vec![Frame::Enter(ROOT as u32)];
        while let Some(frame) = stack.pop() {
            let (mut n, mut entered) = match frame {
                Frame::Enter(n) => (n, false),
                Frame::Visit(n) => (n, true),
            };
            // Follow a run down its right children without the stack.
            loop {
                let node = &self.nodes[n as usize];
                let left = node.left.as_slice();
                if !entered && !left.is_empty() {
                    stack.push(Frame::Visit(n));
                    stack.extend(left.iter().rev().map(|&c| Frame::Enter(c)));
                    break;
                }
                if n as usize != ROOT && !visit(n as usize) {
                    return;
                }
                let [first, rest @ ..] = node.right.as_slice() else { break };
                stack.extend(rest.iter().rev().map(|&c| Frame::Enter(c)));
                (n, entered) = (*first, false);
            }
        }
    }

    /// Every visible node in text order, until `visit` returns false.
    fn walk_visible(&self, mut visit: impl FnMut(&Node) -> bool) {
        self.walk(|i| self.nodes[i].deleted || visit(&self.nodes[i]));
    }

    /// The first node of `n`'s subtree in text order.
    fn leftmost(&self, mut n: usize) -> usize {
        while let Some(&first) = self.nodes[n].left.as_slice().first() {
            n = first as usize;
        }
        n
    }

    pub fn text(&self) -> String {
        let mut out = String::with_capacity(self.visible_bytes);
        self.walk_visible(|node| {
            out.push(node.ch);
            true
        });
        out
    }

    /// The visible text from character `from` up to `to` (or its end).
    pub fn text_between(&self, from: usize, to: usize) -> String {
        let (mut out, mut at) = (String::new(), 0usize);
        self.walk_visible(|node| {
            if at >= from && at < to {
                out.push(node.ch);
            }
            at += 1;
            at < to
        });
        out
    }

    /// Visible characters with their ids, in text order.
    pub fn chars(&self) -> Vec<(CharId, char)> {
        let mut out = Vec::with_capacity(self.visible_chars);
        self.walk_visible(|node| {
            out.push((node.id, node.ch));
            true
        });
        out
    }

    /// The text split into runs by the device that typed them ("who wrote what").
    pub fn spans(&self) -> Vec<(DeviceId, String)> {
        let mut out: Vec<(DeviceId, String)> = Vec::new();
        for (id, ch) in self.chars() {
            match out.last_mut() {
                Some((device, text)) if *device == id.device => text.push(ch),
                _ => out.push((id.device, ch.to_string())),
            }
        }
        out
    }

    /// Who wrote the visible text, in order.
    pub fn authors(&self) -> Vec<Span> {
        let mut out = Vec::new();
        self.walk_visible(|node| {
            push_span(&mut out, node.id.device, 1);
            true
        });
        out
    }

    /// Visible text size in bytes.
    pub fn len_bytes(&self) -> usize {
        self.visible_bytes
    }

    /// Visible text size in characters.
    pub fn len_chars(&self) -> usize {
        self.visible_chars
    }

    /// Edits still waiting for a character that has not arrived.
    pub fn waiting(&self) -> usize {
        self.waiting.values().map(Vec::len).sum()
    }

    /// What the edits recorded in `touched` did to the visible text.
    pub fn change(&self, touched: &Touched) -> TextChange {
        let mut out = TextChange::default();
        if touched.inserted.is_empty() && touched.deleted.is_empty() {
            return out;
        }
        let (mut inserted, mut deleted) = (vec![false; self.nodes.len()], vec![false; self.nodes.len()]);
        touched.inserted.iter().for_each(|&i| inserted[i] = true);
        touched.deleted.iter().for_each(|&i| deleted[i] = true);
        let mut buf = [0u8; 4];
        let mut left = touched.inserted.union(&touched.deleted).count();
        self.walk(|i| {
            let node = &self.nodes[i];
            left -= usize::from(inserted[i] || deleted[i]);
            match (inserted[i], node.deleted) {
                (true, true) => {} // came and went within these edits
                (true, false) => {
                    out.delta.insert(node.ch.encode_utf8(&mut buf));
                    push_span(&mut out.authors, node.id.device, 1);
                }
                (false, true) if deleted[i] => out.delta.delete(1),
                (false, true) => {}
                (false, false) => out.delta.keep(1),
            }
            left > 0 // everything after the last touched character is kept
        });
        out.delta = out.delta.finish();
        out
    }

    /// The change that turns `old`'s visible text into this one's. For a copy
    /// rebuilt after operations were dropped: characters both hold are in the
    /// same order in both, because a character's place depends only on its
    /// ancestors, which it cannot be shown without.
    pub fn change_from(&self, old: &Doc) -> TextChange {
        let (before, after) = (old.chars(), self.chars());
        let held: HashSet<CharId> = before.iter().map(|(id, _)| *id).collect();
        let mut out = TextChange::default();
        let mut buf = [0u8; 4];
        let mut rest = before.iter();
        for (id, ch) in &after {
            if held.contains(id) {
                out.delta.delete(rest.by_ref().take_while(|(b, _)| b != id).count());
                out.delta.keep(1);
            } else {
                out.delta.insert(ch.encode_utf8(&mut buf));
                push_span(&mut out.authors, id.device, 1);
            }
        }
        out.delta.delete(rest.count());
        out.delta = out.delta.finish();
        out
    }

    /// The change that turns this note's visible text back into `old`'s, an
    /// earlier version of it (built from a subset of its edits).
    pub fn delta_to(&self, old: &Doc) -> Delta {
        let mut out = Delta::new();
        let mut buf = [0u8; 4];
        self.walk(|i| {
            let node = &self.nodes[i];
            let was_shown = old.index.get(&node.id).is_some_and(|&o| !old.nodes[o].deleted);
            match (was_shown, !node.deleted) {
                (true, true) => out.keep(1),
                (true, false) => out.insert(node.ch.encode_utf8(&mut buf)),
                (false, true) => out.delete(1),
                (false, false) => {}
            }
            true
        });
        out.finish()
    }

    /// The edit that makes `delta`'s changes to the visible text, by a device
    /// whose next character counter is `next`. Not applied: the caller signs
    /// it, then applies it. A change too big for one edit comes back with the
    /// rest of it, to be made after this one (with a later `next`).
    ///
    /// All of the edit's positions are found in one walk of the note as it is
    /// now. That is sound because a delta's insertions are separated by at
    /// least one character that is kept, so no insertion changes the place
    /// another is attached to.
    pub fn edit_for(&self, next: u64, delta: &Delta) -> Result<(Edit, Option<Delta>), EditError> {
        if delta.reach() > self.visible_chars as u64 {
            return Err(EditError::OutOfRange);
        }
        // The visible characters the change needs to know: from the one before
        // its first insertion or deletion to the last one it covers.
        let to = delta.reach() as usize;
        let from = match delta.steps().first() {
            Some(Step::Keep(n)) => (*n as usize).saturating_sub(1),
            _ => 0,
        };
        let mut nodes: Vec<usize> = Vec::with_capacity(to - from);
        let mut seen = 0usize;
        self.walk(|i| {
            if !self.nodes[i].deleted {
                if seen >= from && seen < to {
                    nodes.push(i);
                }
                seen += 1;
            }
            seen < to
        });
        let visible = Visible { from, nodes };
        // A note over the limit (two big pastes made apart) may still shrink.
        let removed = self.bytes_removed(&visible, delta);
        let grows = delta.inserted_bytes() > removed;
        if grows && self.visible_bytes - removed + delta.inserted_bytes() > MAX_NOTE_BYTES {
            return Err(EditError::TooLarge);
        }

        let mut edit = Edit::default();
        let (mut bytes, mut inserted, mut deleted) = (0usize, 0u64, 0u64);
        // `at` is the next visible character; `made` the same place once this edit is made.
        let (mut at, mut made) = (0usize, 0usize);
        for (k, step) in delta.steps().iter().enumerate() {
            match step {
                Step::Keep(n) => {
                    at += *n as usize;
                    made += *n as usize;
                }
                Step::Insert(text) => {
                    let room = if edit.inserts.len() == MAX_EDIT_PARTS { 0 } else { MAX_EDIT_BYTES - bytes };
                    let now = &text[..longest_prefix(text, room)];
                    if !now.is_empty() {
                        let (parent, side) = self.place(&visible, at);
                        let first = next.checked_add(inserted).ok_or(EditError::TooLarge)?;
                        edit.inserts.push(Run { first, parent, side, text: now.to_string() });
                        bytes += now.len();
                        let n = now.chars().count();
                        inserted += n as u64;
                        made += n;
                    }
                    if now.len() < text.len() {
                        let head = Step::Insert(text[now.len()..].to_string());
                        return Ok((edit, Some(rest_of(made, head, &delta.steps()[k + 1..]))));
                    }
                }
                Step::Delete(n) => {
                    for done in 0..*n {
                        let id = self.nodes[visible.at(at)].id;
                        let extends = edit
                            .deletes
                            .last()
                            .is_some_and(|r| r.device == id.device && r.to.checked_add(1) == Some(id.counter));
                        if deleted == MAX_EDIT_DELETES || (!extends && edit.deletes.len() == MAX_EDIT_PARTS) {
                            let head = Step::Delete(n - done);
                            return Ok((edit, Some(rest_of(made, head, &delta.steps()[k + 1..]))));
                        }
                        match edit.deletes.last_mut() {
                            Some(r) if extends => r.to = id.counter,
                            _ => edit.deletes.push(IdRange { device: id.device, from: id.counter, to: id.counter }),
                        }
                        deleted += 1;
                        at += 1;
                    }
                }
            }
        }
        Ok((edit, None))
    }

    /// Bytes of visible text `delta` deletes (already known to be in range).
    fn bytes_removed(&self, visible: &Visible, delta: &Delta) -> usize {
        let (mut at, mut bytes) = (0usize, 0usize);
        for step in delta.steps() {
            match step {
                Step::Insert(_) => {}
                Step::Keep(n) => at += *n as usize,
                Step::Delete(n) => {
                    let end = at + *n as usize;
                    bytes += (at..end).map(|k| self.nodes[visible.at(k)].ch.len_utf8()).sum::<usize>();
                    at = end;
                }
            }
        }
        bytes
    }

    /// Where text inserted before visible character `at` attaches: Fugue's
    /// rule. The left origin is the visible character before the position (or
    /// the start). If it has no right children, the new text becomes its right
    /// child; otherwise it becomes the left child of the next node after it in
    /// text order, the leftmost node of its first right subtree, which has no
    /// left children.
    fn place(&self, visible: &Visible, at: usize) -> (Option<CharId>, Side) {
        let left_origin = if at == 0 { ROOT } else { visible.at(at - 1) };
        let (parent, side) = match self.nodes[left_origin].right.as_slice().first() {
            None => (left_origin, Side::Right),
            Some(&first) => (self.leftmost(first as usize), Side::Left),
        };
        ((parent != ROOT).then(|| self.nodes[parent].id), side)
    }

    /// The edit that deletes `delete` characters at visible index `at` and
    /// inserts `insert` there ([`Doc::edit_for`], for a change that must fit
    /// one edit).
    pub fn local_edit(&self, next: u64, at: usize, delete: usize, insert: &str) -> Result<Edit, EditError> {
        if at > self.visible_chars {
            return Err(EditError::OutOfRange); // an empty change here would otherwise pass
        }
        match self.edit_for(next, &Delta::splice(at, delete, insert))? {
            (edit, None) => Ok(edit),
            (_, Some(_)) => Err(EditError::TooLarge),
        }
    }
}

/// The nodes of visible characters `from` onwards.
struct Visible {
    from: usize,
    nodes: Vec<usize>,
}

impl Visible {
    fn at(&self, k: usize) -> usize {
        self.nodes[k - self.from]
    }
}

/// Bytes of the longest prefix of `text` of at most `max` bytes.
fn longest_prefix(text: &str, max: usize) -> usize {
    if text.len() <= max {
        return text.len();
    }
    (0..=max).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0)
}

/// What is left of a change after its first part was made: `head`, then the
/// remaining steps, starting `made` characters into the text as it now is.
fn rest_of(made: usize, head: Step, steps: &[Step]) -> Delta {
    let start = Step::Keep(u32::try_from(made).unwrap_or(u32::MAX));
    Delta::from_steps([start, head].into_iter().chain(steps.iter().cloned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceKey;
    use proptest::prelude::*;

    /// A replica: a document, its author and its next counter.
    struct Replica {
        doc: Doc,
        me: DeviceId,
        next: u64,
    }

    impl Replica {
        fn new(n: u64) -> Self {
            Self { doc: Doc::new(), me: DeviceKey::for_test(n).id(), next: 1 }
        }

        /// Edit locally; return the edit to send to the others.
        fn edit(&mut self, at: usize, delete: usize, insert: &str) -> Edit {
            let edit = self.doc.local_edit(self.next, at, delete, insert).unwrap();
            self.next += edit.inserted_chars();
            self.doc.apply(self.me, &edit);
            edit
        }

        fn receive(&mut self, from: DeviceId, edit: &Edit) {
            self.doc.apply(from, edit);
        }
    }

    #[test]
    fn typing_and_deleting_on_one_device_is_plain_editing() {
        let mut a = Replica::new(1);
        a.edit(0, 0, "hello world");
        a.edit(5, 0, ",");
        a.edit(0, 1, "H");
        a.edit(12, 0, "!");
        assert_eq!(a.doc.text(), "Hello, world!");
        a.edit(5, 7, "");
        assert_eq!(a.doc.text(), "Hello!");
        assert_eq!(a.doc.len_bytes(), 6);
    }

    #[test]
    fn concurrent_runs_typed_forwards_do_not_interleave() {
        let (mut a, mut b) = (Replica::new(1), Replica::new(2));
        let base = a.edit(0, 0, "xy");
        b.receive(a.me, &base);
        // Each types a word one key at a time at the same place, apart.
        let ea: Vec<Edit> = "hello".chars().enumerate().map(|(i, c)| a.edit(1 + i, 0, &c.to_string())).collect();
        let eb: Vec<Edit> = "world".chars().enumerate().map(|(i, c)| b.edit(1 + i, 0, &c.to_string())).collect();
        for e in &eb {
            a.receive(b.me, e);
        }
        for e in &ea {
            b.receive(a.me, e);
        }
        assert_eq!(a.doc.text(), b.doc.text());
        assert!(["xhelloworldy", "xworldhelloy"].contains(&a.doc.text().as_str()), "{}", a.doc.text());
    }

    #[test]
    fn concurrent_runs_typed_backwards_do_not_interleave() {
        let (mut a, mut b) = (Replica::new(1), Replica::new(2));
        let base = a.edit(0, 0, "xy");
        b.receive(a.me, &base);
        // Typing backwards: each key goes in front of the previous one.
        let ea: Vec<Edit> = "olleh".chars().map(|c| a.edit(1, 0, &c.to_string())).collect();
        let eb: Vec<Edit> = "dlrow".chars().map(|c| b.edit(1, 0, &c.to_string())).collect();
        for e in eb.iter().rev() {
            a.receive(b.me, e); // and out of order, too
        }
        for e in &ea {
            b.receive(a.me, e);
        }
        assert_eq!(a.doc.text(), b.doc.text());
        assert!(["xhelloworldy", "xworldhelloy"].contains(&a.doc.text().as_str()), "{}", a.doc.text());
    }

    #[test]
    fn an_edit_that_arrives_before_its_parent_waits_for_it() {
        let (mut a, mut b, mut c) = (Replica::new(1), Replica::new(2), Replica::new(3));
        let e1 = a.edit(0, 0, "abc");
        b.receive(a.me, &e1);
        let e2 = b.edit(3, 0, "def"); // attaches to A's 'c'
        let e3 = b.edit(1, 1, ""); // deletes A's 'b'
        c.receive(b.me, &e2);
        c.receive(b.me, &e3);
        assert_eq!(c.doc.text(), "");
        assert_eq!(c.doc.waiting(), 1);
        c.receive(a.me, &e1);
        assert_eq!(c.doc.text(), "acdef");
        assert_eq!(c.doc.waiting(), 0);
        assert_eq!(c.doc.text(), b.doc.text());
    }

    #[test]
    fn who_wrote_what_follows_the_characters() {
        let (mut a, mut b) = (Replica::new(1), Replica::new(2));
        let e = a.edit(0, 0, "hello ");
        b.receive(a.me, &e);
        let e = b.edit(6, 0, "world");
        a.receive(b.me, &e);
        assert_eq!(a.doc.spans(), vec![(a.me, "hello ".to_string()), (b.me, "world".to_string())]);
    }

    #[test]
    fn limits_hold() {
        let a = Replica::new(1);
        assert_eq!(a.doc.local_edit(1, 1, 0, "x"), Err(EditError::OutOfRange));
        assert_eq!(a.doc.local_edit(1, 0, 1, ""), Err(EditError::OutOfRange));
        assert_eq!(a.doc.local_edit(1, 0, 0, &"x".repeat(MAX_EDIT_BYTES + 1)), Err(EditError::TooLarge));
        let huge = Edit { inserts: vec![], deletes: vec![IdRange { device: a.me, from: 0, to: u64::MAX }] };
        assert!(!huge.within_limits(), "a delete range cannot ask for billions of characters");
        let backwards = Edit { inserts: vec![], deletes: vec![IdRange { device: a.me, from: 5, to: 1 }] };
        assert!(!backwards.within_limits());
        let overflow = Edit {
            inserts: vec![Run { first: u64::MAX, parent: None, side: Side::Right, text: "ab".into() }],
            deletes: vec![],
        };
        assert!(!overflow.within_limits());
    }

    /// One random action: which replica, what to do, and where.
    #[derive(Debug, Clone)]
    enum Action {
        Edit { replica: usize, at: u16, delete: u8, insert: String },
        Deliver { from: usize, to: usize },
    }

    fn action() -> impl Strategy<Value = Action> {
        prop_oneof![
            3 => (0usize..3, any::<u16>(), 0u8..4, "[a-z ]{0,4}").prop_map(|(replica, at, delete, insert)| Action::Edit { replica, at, delete, insert }),
            2 => (0usize..3, 0usize..3).prop_map(|(from, to)| Action::Deliver { from, to }),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Three replicas edit at random and exchange edits in random order
        /// (each delivery sends one pending edit, oldest or newest); once
        /// everything is delivered, they hold the same text.
        #[test]
        fn replicas_converge_whatever_the_order(actions in proptest::collection::vec(action(), 1..120), newest_first in any::<bool>()) {
            let mut replicas: Vec<Replica> = (1..=3).map(Replica::new).collect();
            // outbox[from][to]: edits not yet delivered.
            let mut outbox: Vec<Vec<Vec<(DeviceId, Edit)>>> = vec![vec![vec![]; 3]; 3];
            for a in actions {
                match a {
                    Action::Edit { replica, at, delete, insert } => {
                        let len = replica_len(&replicas[replica]);
                        let at = usize::from(at) % (len + 1);
                        let delete = usize::from(delete).min(len - at);
                        if delete == 0 && insert.is_empty() { continue; }
                        let edit = replicas[replica].edit(at, delete, &insert);
                        let me = replicas[replica].me;
                        for (to, queue) in outbox[replica].iter_mut().enumerate() {
                            if to != replica { queue.push((me, edit.clone())); }
                        }
                    }
                    Action::Deliver { from, to } if from != to => {
                        let queue = &mut outbox[from][to];
                        let next = if newest_first { queue.pop() } else if queue.is_empty() { None } else { Some(queue.remove(0)) };
                        if let Some((author, edit)) = next { replicas[to].receive(author, &edit); }
                    }
                    Action::Deliver { .. } => {}
                }
            }
            for queues in &mut outbox {
                for (to, queue) in queues.iter_mut().enumerate() {
                    for (author, edit) in std::mem::take(queue) {
                        replicas[to].receive(author, &edit);
                    }
                }
            }
            let text = replicas[0].doc.text();
            prop_assert_eq!(&replicas[1].doc.text(), &text);
            prop_assert_eq!(&replicas[2].doc.text(), &text);
            for r in &replicas {
                prop_assert_eq!(r.doc.waiting(), 0);
                prop_assert_eq!(r.doc.len_bytes(), text.len());
            }
        }

        /// Each edit's effect on the text, as reported for an open editor, is
        /// exactly its effect, whoever made it and whatever was waiting; and
        /// the change between any two versions is what it claims to be.
        #[test]
        fn reported_changes_are_exact(
            edits in proptest::collection::vec((0usize..2, any::<bool>(), crate::delta::tests::raw()), 1..30),
            order in any::<u64>(),
        ) {
            let mut writers: Vec<Replica> = (1..=2).map(Replica::new).collect();
            let mut made: Vec<(DeviceId, Edit)> = Vec::new();
            let mut held_back: Vec<(DeviceId, Edit)> = Vec::new();
            for (w, shared, raw) in &edits {
                let w = &mut writers[*w];
                let delta = crate::delta::tests::delta_for(w.doc.len_chars(), raw);
                let before = w.doc.text();
                let (edit, rest) = w.doc.edit_for(w.next, &delta).unwrap();
                prop_assert!(rest.is_none());
                w.next += edit.inserted_chars();
                let mut touched = Touched::default();
                w.doc.apply_tracked(w.me, &edit, &mut touched);
                prop_assert_eq!(w.doc.text(), delta.apply(&before).unwrap(), "the edit makes the delta");
                prop_assert_eq!(w.doc.change(&touched).delta.apply(&before).unwrap(), w.doc.text());
                held_back.push((w.me, edit.clone()));
                made.push((w.me, edit));
                if *shared {
                    // Both writers catch up, so later edits build on each other's.
                    for (author, edit) in held_back.drain(..) {
                        for w in &mut writers { w.receive(author, &edit); }
                    }
                }
            }
            // A third device receives them shuffled, in small batches.
            let mut doc = Doc::new();
            let mut versions: Vec<Doc> = Vec::new();
            let mut seed = order | 1;
            let mut authors: Vec<DeviceId> = Vec::new();
            while !made.is_empty() {
                let before = doc.clone();
                let mut touched = Touched::default();
                for _ in 0..=(seed % 3) {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    if made.is_empty() { break; }
                    let (author, edit) = made.swap_remove((seed >> 33) as usize % made.len());
                    doc.apply_tracked(author, &edit, &mut touched);
                }
                let change = doc.change(&touched);
                prop_assert_eq!(change.delta.apply(&before.text()).unwrap(), doc.text());
                prop_assert_eq!(&doc.change_from(&before).delta.apply(&before.text()).unwrap(), &doc.text());
                prop_assert_eq!(before.change_from(&doc).delta.apply(&doc.text()).unwrap(), before.text(), "and backwards");
                // An editor that keeps authors from the changes it is told agrees with the note.
                prop_assert_eq!(change.authors.iter().map(|s| s.chars as usize).sum::<usize>(), change.delta.inserted_chars());
                let mut written = change.authors.iter().flat_map(|s| std::iter::repeat(s.device).take(s.chars as usize));
                let mut at = 0;
                for step in change.delta.steps() {
                    match step {
                        Step::Keep(n) => at += *n as usize,
                        Step::Delete(n) => { authors.drain(at..at + *n as usize); }
                        Step::Insert(t) => for _ in t.chars() { authors.insert(at, written.next().unwrap()); at += 1; },
                    }
                }
                let held: Vec<DeviceId> = doc.authors().iter().flat_map(|s| std::iter::repeat(s.device).take(s.chars as usize)).collect();
                prop_assert_eq!(&authors, &held);
                versions.push(before);
            }
            prop_assert_eq!(doc.waiting(), 0);
            for old in &versions {
                prop_assert_eq!(doc.delta_to(old).apply(&doc.text()).unwrap(), old.text());
            }
        }

        /// On one device, the document behaves exactly like a string.
        #[test]
        fn one_replica_matches_a_plain_string(edits in proptest::collection::vec((any::<u16>(), 0u8..5, "[a-zé]{0,5}"), 1..80)) {
            let mut r = Replica::new(1);
            let mut model: Vec<char> = Vec::new();
            for (at, delete, insert) in edits {
                let at = usize::from(at) % (model.len() + 1);
                let delete = usize::from(delete).min(model.len() - at);
                r.edit(at, delete, &insert);
                model.splice(at..at + delete, insert.chars());
            }
            prop_assert_eq!(r.doc.text(), model.into_iter().collect::<String>());
        }
    }

    /// Make `delta` on `doc` the way the daemon does: in as many edits as it takes.
    fn make(doc: &mut Doc, me: DeviceId, mut seq: u64, delta: &Delta) -> usize {
        let mut rest = Some(delta.clone());
        let mut edits = 0;
        while let Some(d) = rest {
            let (edit, more) = doc.edit_for(first_counter(seq).unwrap(), &d).unwrap();
            assert!(edit.within_limits() && edit.ids_belong_to(seq), "every part is a valid edit");
            assert!(!edit.is_empty(), "every part makes progress");
            doc.apply(me, &edit);
            rest = more;
            seq += 1;
            edits += 1;
        }
        edits
    }

    #[test]
    fn a_big_change_is_made_in_several_valid_edits() {
        let me = DeviceKey::for_test(1).id();
        let mut doc = Doc::new();
        // Multi-byte characters straddle the 4 KiB boundaries.
        let text = "é漢x".repeat(10_000);
        assert_eq!(make(&mut doc, me, 1, &Delta::splice(0, 0, &text)), text.len().div_ceil(MAX_EDIT_BYTES - 1));
        assert_eq!(doc.text(), text);
        // Delete and replace most of it in one change with several hunks.
        let mut d = Delta::new();
        d.keep(3);
        d.delete(12_000);
        d.insert(&"new ".repeat(3000));
        d.keep(100);
        d.delete(5_000);
        d.insert("end");
        let d = d.finish();
        let expected = d.apply(&text).unwrap();
        make(&mut doc, me, 100, &d);
        assert_eq!(doc.text(), expected);
        assert_eq!(doc.len_chars(), expected.chars().count());
    }

    #[test]
    fn deleting_text_typed_key_by_key_stays_within_the_range_limit() {
        let me = DeviceKey::for_test(1).id();
        let mut doc = Doc::new();
        for seq in 1..=1500u64 {
            make(&mut doc, me, seq, &Delta::splice(seq as usize - 1, 0, "k"));
        }
        // 1500 ranges (each key has its own operation), 1024 to an edit.
        assert_eq!(make(&mut doc, me, 2000, &Delta::splice(0, 1500, "")), 2);
        assert_eq!(doc.text(), "");
    }

    #[test]
    fn a_note_over_the_limit_may_shrink_but_not_grow() {
        let (a, b) = (DeviceKey::for_test(1).id(), DeviceKey::for_test(2).id());
        let mut doc = Doc::new();
        let part = "x".repeat(MAX_EDIT_BYTES);
        // Two devices fill the note apart; merged, it is over the limit.
        for (author, seq) in (1..=40u64).flat_map(|seq| [(a, seq), (b, seq)]) {
            let run = Run { first: first_counter(seq).unwrap(), parent: None, side: Side::Right, text: part.clone() };
            doc.apply(author, &Edit { inserts: vec![run], deletes: vec![] });
        }
        assert!(doc.len_bytes() > MAX_NOTE_BYTES);
        assert_eq!(doc.edit_for(first_counter(99).unwrap(), &Delta::splice(0, 0, "y")), Err(EditError::TooLarge));
        assert!(doc.edit_for(first_counter(99).unwrap(), &Delta::splice(0, 10, "y")).is_ok());
        assert_eq!(doc.edit_for(1, &Delta::splice(doc.len_chars() + 1, 0, "y")), Err(EditError::OutOfRange));
    }

    #[test]
    fn going_back_to_an_earlier_version() {
        let (mut a, mut b) = (Replica::new(1), Replica::new(2));
        let e = a.edit(0, 0, "keep this, drop that");
        b.receive(a.me, &e);
        let old = a.doc.clone();
        let e = b.edit(9, 11, "!");
        a.receive(b.me, &e);
        a.edit(0, 4, "KEEP");
        assert_eq!(a.doc.text(), "KEEP this!");
        let back = a.doc.delta_to(&old);
        assert_eq!(back.apply(&a.doc.text()).unwrap(), "keep this, drop that");
        // Characters shown in both stay the same characters.
        assert!(back.steps().contains(&Step::Keep(5)), "{back:?}");
    }

    fn replica_len(r: &Replica) -> usize {
        r.doc.chars().len()
    }
}
