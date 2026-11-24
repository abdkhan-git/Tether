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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// The position or length is outside the note.
    OutOfRange,
    /// The edit, or the note after it, would exceed a limit.
    TooLarge,
}

const ROOT: usize = 0;

#[derive(Clone, Debug)]
struct Node {
    id: CharId,
    ch: char,
    deleted: bool,
    left: Vec<usize>,
    right: Vec<usize>,
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
}

impl Default for Doc {
    fn default() -> Self {
        let root = Node {
            id: CharId { counter: 0, device: DeviceId([0; 32]) },
            ch: '\0',
            deleted: true,
            left: vec![],
            right: vec![],
        };
        Self {
            nodes: vec![root],
            index: HashMap::new(),
            waiting: HashMap::new(),
            early_deletes: HashSet::new(),
            visible_bytes: 0,
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
        let mut queue: VecDeque<(DeviceId, Run)> = edit.inserts.iter().map(|r| (author, r.clone())).collect();
        while let Some((author, run)) = queue.pop_front() {
            self.insert_run(author, run, &mut queue);
        }
        for range in &edit.deletes {
            for counter in range.from..=range.to {
                let id = CharId { counter, device: range.device };
                match self.index.get(&id) {
                    Some(&i) => self.mark_deleted(i),
                    None => {
                        self.early_deletes.insert(id);
                    }
                }
            }
        }
    }

    fn mark_deleted(&mut self, i: usize) {
        let node = &mut self.nodes[i];
        if !node.deleted {
            node.deleted = true;
            self.visible_bytes -= node.ch.len_utf8();
        }
    }

    fn insert_run(&mut self, author: DeviceId, run: Run, queue: &mut VecDeque<(DeviceId, Run)>) {
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
            self.nodes.push(Node { id, ch, deleted, left: vec![], right: vec![] });
            self.index.insert(id, i);
            if !deleted {
                self.visible_bytes += ch.len_utf8();
            }
            let siblings = match attach.1 {
                Side::Left => &self.nodes[attach.0].left,
                Side::Right => &self.nodes[attach.0].right,
            };
            let at = siblings.partition_point(|&s| self.nodes[s].id < id);
            match attach.1 {
                Side::Left => self.nodes[attach.0].left.insert(at, i),
                Side::Right => self.nodes[attach.0].right.insert(at, i),
            }
            attach = (i, Side::Right);
            if let Some(runs) = self.waiting.remove(&id) {
                queue.extend(runs);
            }
        }
    }

    /// Every node in text order (tombstones included), the root excluded.
    /// Iterative: a long run is a long chain, too deep to recurse.
    fn in_order(&self) -> Vec<usize> {
        enum Frame {
            Enter(usize),
            Visit(usize),
        }
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack = vec![Frame::Enter(ROOT)];
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Enter(n) => {
                    let node = &self.nodes[n];
                    stack.extend(node.right.iter().rev().map(|&c| Frame::Enter(c)));
                    stack.push(Frame::Visit(n));
                    stack.extend(node.left.iter().rev().map(|&c| Frame::Enter(c)));
                }
                Frame::Visit(n) if n != ROOT => out.push(n),
                Frame::Visit(_) => {}
            }
        }
        out
    }

    /// The first node of `n`'s subtree in text order.
    fn leftmost(&self, mut n: usize) -> usize {
        while let Some(&first) = self.nodes[n].left.first() {
            n = first;
        }
        n
    }

    pub fn text(&self) -> String {
        self.in_order().into_iter().filter(|&i| !self.nodes[i].deleted).map(|i| self.nodes[i].ch).collect()
    }

    /// Visible characters with their ids, in text order.
    pub fn chars(&self) -> Vec<(CharId, char)> {
        self.in_order()
            .into_iter()
            .filter(|&i| !self.nodes[i].deleted)
            .map(|i| (self.nodes[i].id, self.nodes[i].ch))
            .collect()
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

    /// Visible text size in bytes.
    pub fn len_bytes(&self) -> usize {
        self.visible_bytes
    }

    /// Edits still waiting for a character that has not arrived.
    pub fn waiting(&self) -> usize {
        self.waiting.values().map(Vec::len).sum()
    }

    /// The edit that deletes `delete` characters at visible index `at` and
    /// inserts `insert` there, by a device whose next character counter
    /// is `next`. Not applied: the caller signs it, then applies it.
    pub fn local_edit(&self, next: u64, at: usize, delete: usize, insert: &str) -> Result<Edit, EditError> {
        let visible: Vec<usize> = self.in_order().into_iter().filter(|&i| !self.nodes[i].deleted).collect();
        if at > visible.len() || delete > visible.len() - at {
            return Err(EditError::OutOfRange);
        }
        let removed: usize = visible[at..at + delete].iter().map(|&i| self.nodes[i].ch.len_utf8()).sum();
        if insert.len() > MAX_EDIT_BYTES || self.visible_bytes - removed + insert.len() > MAX_NOTE_BYTES {
            return Err(EditError::TooLarge);
        }

        let mut edit = Edit::default();
        // Deletes, as ranges of consecutive counters by the same author.
        for &i in &visible[at..at + delete] {
            let id = self.nodes[i].id;
            match edit.deletes.last_mut() {
                Some(r) if r.device == id.device && r.to.checked_add(1) == Some(id.counter) => r.to = id.counter,
                _ => edit.deletes.push(IdRange { device: id.device, from: id.counter, to: id.counter }),
            }
        }
        if edit.deletes.len() > MAX_EDIT_PARTS {
            return Err(EditError::TooLarge);
        }

        if !insert.is_empty() {
            // Fugue's rule. The left origin is the visible character before
            // the position (or the start). If it has no right children, the
            // new text becomes its right child; otherwise it becomes the left
            // child of the next node after it in text order, the leftmost
            // node of its first right subtree, which has no left children.
            let left_origin = if at == 0 { ROOT } else { visible[at - 1] };
            let (parent, side) = if self.nodes[left_origin].right.is_empty() {
                (left_origin, Side::Right)
            } else {
                (self.leftmost(self.nodes[left_origin].right[0]), Side::Left)
            };
            let parent = (parent != ROOT).then(|| self.nodes[parent].id);
            edit.inserts.push(Run { first: next, parent, side, text: insert.to_string() });
        }
        Ok(edit)
    }
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

    fn replica_len(r: &Replica) -> usize {
        r.doc.chars().len()
    }
}
