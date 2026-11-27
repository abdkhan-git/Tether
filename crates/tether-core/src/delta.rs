//! Text changes by position, the language an editor speaks.
//!
//! Between devices, note edits name characters by permanent id
//! ([`crate::notes`]). Between the daemon and an editor window on the same
//! device they are [`Delta`]s: walk the text, keeping, deleting and inserting.
//! An editor's delta is made against the version of the text it last saw; if
//! the daemon has applied others since, it first carries the delta over them
//! with [`transform`]. The editor does the same in the other direction for
//! changes that reach it while its own are on their way.
//!
//! This is operational transformation in its simplest, two-party form, with
//! the daemon deciding the order. The one property it needs is TP1: for two
//! changes to the same text, applying one and then the other carried over it
//! gives the same text either way round. `app/src/lib/notes/delta.ts` mirrors
//! this file, and both are checked against the same generated cases
//! (`testdata/delta-cases.json`).
//!
//! Positions and lengths count Unicode scalar values (Rust `char`s).

use serde::{Deserialize, Serialize};

/// Most steps in one delta.
pub const MAX_STEPS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    /// Leave this many characters as they are.
    Keep(u32),
    /// Remove this many characters.
    Delete(u32),
    /// Insert this text here.
    Insert(String),
}

/// A change to a text, start to end. Whatever the steps do not reach is kept.
///
/// Built with the methods below it is always in its simplest form: no empty
/// steps, no two neighbours of the same kind, an insertion before a deletion
/// at the same place, and no keep at the end.
/// Decoding one puts it in that form.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<Step>")]
pub struct Delta(Vec<Step>);

impl From<Vec<Step>> for Delta {
    fn from(steps: Vec<Step>) -> Self {
        Self::from_steps(steps)
    }
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

/// The byte offset of character `n` of `s` (or its end).
fn byte_at(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map_or(s.len(), |(i, _)| i)
}

impl Delta {
    pub fn new() -> Self {
        Self::default()
    }

    /// From steps of any shape (such as ones that arrived over a channel).
    pub fn from_steps(steps: impl IntoIterator<Item = Step>) -> Self {
        let mut d = Self::new();
        for s in steps {
            match s {
                Step::Keep(n) => d.keep(n as usize),
                Step::Delete(n) => d.delete(n as usize),
                Step::Insert(t) => d.insert(&t),
            }
        }
        d.finish()
    }

    /// Replace `delete` characters at `at` with `insert`.
    pub fn splice(at: usize, delete: usize, insert: &str) -> Self {
        let mut d = Self::new();
        d.keep(at);
        d.insert(insert);
        d.delete(delete);
        d.finish()
    }

    pub fn keep(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        match self.0.last_mut() {
            Some(Step::Keep(k)) => *k = k.saturating_add(count(n)),
            _ => self.0.push(Step::Keep(count(n))),
        }
    }

    pub fn delete(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        match self.0.last_mut() {
            Some(Step::Delete(k)) => *k = k.saturating_add(count(n)),
            _ => self.0.push(Step::Delete(count(n))),
        }
    }

    pub fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.0.as_mut_slice() {
            [.., Step::Insert(t)] | [.., Step::Insert(t), Step::Delete(_)] => t.push_str(text),
            [.., Step::Delete(_)] => {
                let at = self.0.len() - 1;
                self.0.insert(at, Step::Insert(text.to_string()));
            }
            _ => self.0.push(Step::Insert(text.to_string())),
        }
    }

    /// Drop a keep at the end: it says nothing.
    #[must_use]
    pub fn finish(mut self) -> Self {
        if let Some(Step::Keep(_)) = self.0.last() {
            self.0.pop();
        }
        self
    }

    pub fn steps(&self) -> &[Step] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Characters of the original text this delta covers (at least).
    pub fn reach(&self) -> u64 {
        self.0.iter().map(|s| if let Step::Keep(n) | Step::Delete(n) = s { u64::from(*n) } else { 0 }).sum()
    }

    pub fn inserted_bytes(&self) -> usize {
        self.0.iter().map(|s| if let Step::Insert(t) = s { t.len() } else { 0 }).sum()
    }

    pub fn inserted_chars(&self) -> usize {
        self.0.iter().map(|s| if let Step::Insert(t) = s { chars(t) } else { 0 }).sum()
    }

    pub fn deleted_chars(&self) -> u64 {
        self.0.iter().map(|s| if let Step::Delete(n) = s { u64::from(*n) } else { 0 }).sum()
    }

    /// The changed text, or `None` if the delta reaches past the end of it.
    pub fn apply(&self, text: &str) -> Option<String> {
        let mut out = String::with_capacity(text.len() + self.inserted_bytes());
        let mut rest = text;
        for step in &self.0 {
            match step {
                Step::Insert(t) => out.push_str(t),
                Step::Keep(n) | Step::Delete(n) => {
                    let n = *n as usize;
                    let end = byte_at(rest, n);
                    if end == rest.len() && chars(rest) < n {
                        return None;
                    }
                    if let Step::Keep(_) = step {
                        out.push_str(&rest[..end]);
                    }
                    rest = &rest[end..];
                }
            }
        }
        out.push_str(rest);
        Some(out)
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Where a walk over a delta has got to: steps taken one part at a time, with
/// the endless keep after the last one.
struct Walk<'a> {
    steps: &'a [Step],
    /// How much of `steps[0]` is already used (characters).
    used: usize,
}

#[derive(Clone, Copy)]
enum Head<'a> {
    Keep(usize),
    Delete(usize),
    Insert(&'a str),
    /// Nothing but the rest of the text.
    End,
}

impl<'a> Walk<'a> {
    fn new(d: &'a Delta) -> Self {
        Self { steps: &d.0, used: 0 }
    }

    fn head(&self) -> Head<'a> {
        match self.steps.first() {
            None => Head::End,
            Some(Step::Keep(n)) => Head::Keep(*n as usize - self.used),
            Some(Step::Delete(n)) => Head::Delete(*n as usize - self.used),
            Some(Step::Insert(t)) => Head::Insert(t),
        }
    }

    fn take(&mut self, n: usize) {
        match self.steps.first() {
            None => {}
            Some(Step::Insert(_)) => self.steps = &self.steps[1..],
            Some(Step::Keep(k) | Step::Delete(k)) => {
                self.used += n;
                if self.used >= *k as usize {
                    self.steps = &self.steps[1..];
                    self.used = 0;
                }
            }
        }
    }
}

/// Carry `edit` and `applied`, two changes to the same text, over each other.
/// Returns `(edit after applied, applied after edit)`. Where both insert at
/// the same place, `applied`'s text comes first.
pub fn transform(edit: &Delta, applied: &Delta) -> (Delta, Delta) {
    let (mut e, mut a) = (Walk::new(edit), Walk::new(applied));
    let (mut e2, mut a2) = (Delta::new(), Delta::new());
    loop {
        match (e.head(), a.head()) {
            // The change already applied inserts first.
            (_, Head::Insert(t)) => {
                e2.keep(chars(t));
                a2.insert(t);
                a.take(0);
            }
            (Head::Insert(t), _) => {
                e2.insert(t);
                a2.keep(chars(t));
                e.take(0);
            }
            (Head::End, Head::End) => break,
            (eh, ah) => {
                let left = |h: Head| match h {
                    Head::Keep(n) | Head::Delete(n) => n,
                    _ => usize::MAX,
                };
                let n = left(eh).min(left(ah));
                match (eh, ah) {
                    // Both remove it: nothing left for either to do.
                    (Head::Delete(_), Head::Delete(_)) => {}
                    (Head::Delete(_), _) => e2.delete(n),
                    (_, Head::Delete(_)) => a2.delete(n),
                    _ => {
                        e2.keep(n);
                        a2.keep(n);
                    }
                }
                e.take(n);
                a.take(n);
            }
        }
    }
    (e2.finish(), a2.finish())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn applying_and_building() {
        assert_eq!(Delta::splice(1, 3, "EL").apply("hello").unwrap(), "hELo");
        assert_eq!(Delta::splice(5, 0, "!").apply("hello").unwrap(), "hello!");
        assert_eq!(Delta::splice(6, 0, "!").apply("hello"), None, "past the end");
        assert_eq!(Delta::splice(4, 2, "").apply("hello"), None);
        assert_eq!(Delta::splice(1, 1, "é").apply("a😀b").unwrap(), "aéb");
        assert!(Delta::splice(3, 0, "").is_empty(), "a change that does nothing is empty");
        let mut d = Delta::new();
        d.delete(1);
        d.insert("a");
        d.delete(1);
        d.insert("b");
        d.keep(9);
        assert_eq!(d.finish().steps(), [Step::Insert("ab".into()), Step::Delete(2)]);
        assert_eq!(
            Delta::from_steps([Step::Keep(0), Step::Keep(1), Step::Keep(1), Step::Delete(1), Step::Insert("x".into())])
                .steps(),
            [Step::Keep(2), Step::Insert("x".into()), Step::Delete(1)]
        );
    }

    #[test]
    fn at_the_same_place_the_change_already_applied_comes_first() {
        let mine = Delta::splice(1, 0, "M");
        let theirs = Delta::splice(1, 0, "T");
        let (mine2, theirs2) = transform(&mine, &theirs);
        assert_eq!(mine2.apply(&theirs.apply("ab").unwrap()).unwrap(), "aTMb");
        assert_eq!(theirs2.apply(&mine.apply("ab").unwrap()).unwrap(), "aTMb");
    }

    #[test]
    fn huge_counts_do_not_overflow() {
        let d = Delta::from_steps([Step::Keep(u32::MAX), Step::Keep(u32::MAX), Step::Insert("x".into())]);
        assert_eq!(d.steps()[0], Step::Keep(u32::MAX));
        assert_eq!(d.apply("abc"), None);
        let _ = transform(&d, &Delta::splice(1, 1, "y"));
    }

    /// A text and a delta that fits it.
    pub(crate) fn delta_for(len: usize, raw: &[(u8, u8, String)]) -> Delta {
        let mut d = Delta::new();
        let mut left = len;
        for (kind, n, text) in raw {
            let n = usize::from(*n).min(left);
            match kind % 3 {
                0 => {
                    d.keep(n);
                    left -= n;
                }
                1 => {
                    d.delete(n);
                    left -= n;
                }
                _ => d.insert(text),
            }
        }
        d.finish()
    }

    pub(crate) fn raw() -> impl Strategy<Value = Vec<(u8, u8, String)>> {
        proptest::collection::vec((any::<u8>(), 0u8..6, "[a-z😀é]{1,4}"), 0..10)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1024))]

        /// TP1: either order gives the same text.
        #[test]
        fn both_orders_agree(text in "[A-Z😀]{0,20}", a in raw(), b in raw()) {
            let n = text.chars().count();
            let (a, b) = (delta_for(n, &a), delta_for(n, &b));
            let (a2, b2) = transform(&a, &b);
            let one = a2.apply(&b.apply(&text).unwrap()).unwrap();
            let two = b2.apply(&a.apply(&text).unwrap()).unwrap();
            prop_assert_eq!(one, two);
        }

        /// Built deltas are in their simplest form, so equal changes are equal values.
        #[test]
        fn rebuilding_changes_nothing(raw in raw()) {
            let d = delta_for(20, &raw);
            prop_assert_eq!(Delta::from_steps(d.steps().to_vec()), d);
        }
    }

    /// Cases shared with the TypeScript mirror (app/src/lib/notes/delta.test.ts).
    /// The file is generated here and checked in; if this test fails after a
    /// deliberate change, set TETHER_WRITE_CASES=1 and run it again.
    #[test]
    fn the_shared_cases_are_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/delta-cases.json");
        let made = cases();
        if std::env::var_os("TETHER_WRITE_CASES").is_some() {
            std::fs::create_dir_all(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata")).unwrap();
            std::fs::write(path, &made).unwrap();
        }
        let held = std::fs::read_to_string(path).expect("testdata/delta-cases.json").replace("\r\n", "\n");
        assert!(held == made, "delta-cases.json is out of date");
    }

    fn cases() -> String {
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        const LETTERS: [char; 8] = ['a', 'b', 'c', 'é', '😀', 'x', ' ', '漢'];
        let word = |n: &mut dyn FnMut(u64) -> u64, max: u64| -> String {
            (0..n(max + 1)).map(|_| LETTERS[n(8) as usize]).collect()
        };
        let json = |d: &Delta| {
            let parts: Vec<String> = d
                .steps()
                .iter()
                .map(|s| match s {
                    Step::Keep(n) => n.to_string(),
                    Step::Delete(n) => format!("-{n}"),
                    Step::Insert(t) => format!("\"{t}\""),
                })
                .collect();
            format!("[{}]", parts.join(","))
        };
        let mut out = Vec::new();
        for _ in 0..300 {
            let text = word(&mut next, 12);
            let len = text.chars().count();
            let mut delta = || {
                let raw: Vec<_> = (0..next(6))
                    .map(|_| (next(3) as u8, next(5) as u8, word(&mut next, 3)))
                    .filter(|(_, _, t)| !t.is_empty())
                    .collect();
                delta_for(len, &raw)
            };
            let (edit, applied) = (delta(), delta());
            let (edit2, applied2) = transform(&edit, &applied);
            let result = edit2.apply(&applied.apply(&text).unwrap()).unwrap();
            out.push(format!(
                "{{\"text\":\"{text}\",\"edit\":{},\"applied\":{},\"edit2\":{},\"applied2\":{},\"result\":\"{result}\"}}",
                json(&edit),
                json(&applied),
                json(&edit2),
                json(&applied2)
            ));
        }
        format!("[\n{}\n]\n", out.join(",\n"))
    }
}
