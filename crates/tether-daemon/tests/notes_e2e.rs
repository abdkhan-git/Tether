//! End-to-end tests of notes: real `tetherd` processes over loopback QUIC,
//! driven through the control channel as the app does it.
//! Threat model: docs/SECURITY.md, "Notes".

mod common;

use std::time::{Duration, Instant};

use common::{Control, Daemon};
use tether_core::{Delta, DeviceId, NoteRef};
use tether_ipc::{Client, EditorId, Event, Events, NoteEdit, NoteSummary, NoteText, Reply, Request};
use tokio::time::timeout;

async fn client(c: &Control) -> Client {
    Client::connect_to(&c.endpoint, &c.token).await.expect("authenticated client")
}

async fn notes(c: &mut Client) -> Vec<NoteSummary> {
    match c.request(&Request::Notes).await.unwrap() {
        Reply::Notes(n) => n,
        other => panic!("unexpected {other:?}"),
    }
}

async fn open(c: &mut Client, note: NoteRef, editor: EditorId) -> NoteText {
    match c.request(&Request::NoteOpen { note, editor: Some(editor) }).await.unwrap() {
        Reply::NoteText(t) => t,
        other => panic!("unexpected {other:?}"),
    }
}

async fn create(c: &mut Client, title: &str) -> NoteRef {
    match c.request(&Request::NoteCreate { title: title.into(), notebook: "Work".into() }).await.unwrap() {
        Reply::NoteCreated { note } => note,
        other => panic!("unexpected {other:?}"),
    }
}

/// Wait until `f` holds of the list of notes.
async fn wait_for_notes(c: &mut Client, f: impl Fn(&[NoteSummary]) -> bool) -> Vec<NoteSummary> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let list = notes(c).await;
        if f(&list) || Instant::now() > deadline {
            return list;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// An editor that waits for each edit to be answered before the next.
struct Editor {
    id: EditorId,
    note: NoteRef,
    version: u64,
    text: String,
    n: u64,
}

impl Editor {
    async fn open(c: &mut Client, note: NoteRef, id: u8) -> Self {
        let t = open(c, note, [id; 16]).await;
        Self { id: [id; 16], note, version: t.version, text: t.text, n: 0 }
    }

    fn edit(&mut self, delta: Delta) -> Request {
        self.n += 1;
        let Editor { id, note, version, n, .. } = *self;
        Request::NoteEdit(NoteEdit { note, base: version, delta, editor: id, n, allow_secret: false })
    }

    /// Take in an event; true if it answers this editor's last edit.
    fn sees(&mut self, event: &Event) -> bool {
        let Event::NotePatch { note, from, to, change, by } = event else { return false };
        if *note != self.note || *to <= self.version {
            return false;
        }
        assert_eq!(*from, self.version, "patches follow one another");
        self.text = change.delta.apply(&self.text).expect("the patch fits");
        self.version = *to;
        *by == Some((self.id, self.n))
    }

    async fn until(&mut self, events: &mut Events, done: impl Fn(&Self, bool) -> bool) {
        timeout(Duration::from_secs(20), async {
            loop {
                let answered = self.sees(&events.next().await.unwrap());
                if done(self, answered) {
                    return;
                }
            }
        })
        .await
        .expect("the editor hears what it waits for");
    }

    /// Type `count` characters one at a time, at the start, middle and end.
    async fn type_keys(&mut self, c: &mut Client, events: &mut Events, key: &str, count: usize) {
        for k in 0..count {
            let len = self.text.chars().count();
            let req = self.edit(Delta::splice([0, len / 2, len][k % 3], 0, key));
            assert_eq!(c.request(&req).await.unwrap(), Reply::NoteEdited);
            self.until(events, |_, answered| answered).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_devices_type_in_one_note_at_once_and_agree() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, ctl_b) = Daemon::with_control("bravo");
    common::pair(&mut b, &mut a);
    let (mut ca, mut cb) = (client(&ctl_a).await, client(&ctl_b).await);
    let mut events_a = client(&ctl_a).await.subscribe().await.unwrap();
    let mut events_b = client(&ctl_b).await.subscribe().await.unwrap();

    let note = create(&mut ca, "Shared").await;
    let list = wait_for_notes(&mut cb, |l| l.iter().any(|n| n.note == note && n.notebook == "Work")).await;
    let there = list.iter().find(|n| n.note == note).expect("the note reaches B");
    assert_eq!((there.title.as_str(), there.notebook.as_str(), there.pinned), ("Shared", "Work", false));

    let mut ea = Editor::open(&mut ca, note, 1).await;
    let mut eb = Editor::open(&mut cb, note, 2).await;
    tokio::join!(ea.type_keys(&mut ca, &mut events_a, "a", 60), eb.type_keys(&mut cb, &mut events_b, "b", 60));

    // Once everything has been delivered, both editors show the same 120 characters.
    let all = |e: &Editor, _| e.text.len() == 120;
    if ea.text.len() < 120 {
        ea.until(&mut events_a, all).await;
    }
    if eb.text.len() < 120 {
        eb.until(&mut events_b, all).await;
    }
    assert_eq!(ea.text, eb.text);
    assert_eq!(ea.text.matches('a').count(), 60, "nothing lost");
    let typed = ea.text.clone();
    let (ta, tb) = (open(&mut ca, note, [9; 16]).await, open(&mut cb, note, [9; 16]).await);
    assert_eq!((&ta.text, &ta.authors), (&ea.text, &tb.authors));
    let a_id: DeviceId = a.id.parse().unwrap();
    let by_a: u32 = ta.authors.iter().filter(|s| s.device == a_id).map(|s| s.chars).sum();
    assert_eq!(by_a, 60, "each character keeps its author");

    // Both see one history, and B can restore a version A wrote in.
    let history = |r: Reply| match r {
        Reply::NoteHistory(h) => h,
        other => panic!("unexpected {other:?}"),
    };
    let ha = history(ca.request(&Request::NoteHistory { note }).await.unwrap());
    let hb = history(cb.request(&Request::NoteHistory { note }).await.unwrap());
    assert_eq!(ha, hb);
    let old = ha.last().expect("there is history");
    let restore = Request::NoteRestore { note, at: old.at, author: old.author };
    assert_eq!(cb.request(&restore).await.unwrap(), Reply::NoteChanged);
    let Reply::NoteText(then) = ca.request(&Request::NoteAt { note, at: old.at, author: old.author }).await.unwrap()
    else {
        panic!("not a text")
    };
    ea.until(&mut events_a, |e, _| e.text == then.text).await;

    // Details changed on B show on A.
    let meta =
        Request::NoteMeta { note, title: Some("Renamed".into()), notebook: None, pinned: Some(true), trashed: None };
    assert_eq!(cb.request(&meta).await.unwrap(), Reply::NoteChanged);
    let list = wait_for_notes(&mut ca, |l| l.iter().any(|n| n.note == note && n.pinned)).await;
    assert_eq!(list.iter().find(|n| n.note == note).unwrap().title, "Renamed");

    // Nothing anyone typed reaches either daemon's log output.
    for d in [&a, &b] {
        assert!(!d.since(0).contains(&typed[..40]) && !d.since(0).contains("Renamed"), "note text was logged");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_secret_stays_here_until_the_user_says_otherwise() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, ctl_b) = Daemon::with_control("bravo");
    common::pair(&mut b, &mut a);
    let (mut ca, mut cb) = (client(&ctl_a).await, client(&ctl_b).await);
    let mut e = Editor::open(&mut ca, NoteRef::SCRATCHPAD, 1).await;

    let req = e.edit(Delta::splice(0, 0, "key AKIAIOSFODNN7EXAMPLE"));
    let kind = "AWS access key".to_string();
    assert_eq!(ca.request(&req).await.unwrap(), Reply::NoteSecret { kind });
    common::settle();
    let held = |list: &[NoteSummary]| list.iter().find(|n| n.note == NoteRef::SCRATCHPAD).unwrap().chars;
    assert_eq!(held(&notes(&mut ca).await), 0, "not recorded here");
    assert_eq!(held(&notes(&mut cb).await), 0, "and not sent");

    let Request::NoteEdit(edit) = req else { unreachable!() };
    let allowed = Request::NoteEdit(NoteEdit { allow_secret: true, ..edit });
    assert_eq!(ca.request(&allowed).await.unwrap(), Reply::NoteEdited);
    let list = wait_for_notes(&mut cb, |l| held(l) > 0).await;
    assert_eq!(held(&list), 24);
    assert!(!a.since(0).contains("AKIA") && !b.since(0).contains("AKIA"), "never logged");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_note_fits_and_edits_are_paced() {
    let (_a, ctl) = Daemon::with_control("alpha");
    let mut c = client(&ctl).await;
    let mut events = client(&ctl).await.subscribe().await.unwrap();
    let mut e = Editor::open(&mut c, NoteRef::SCRATCHPAD, 1).await;
    let text = "0123456789abcde\n".repeat(16 << 10); // 256 KiB
    let req = e.edit(Delta::splice(0, 0, &text));
    assert_eq!(c.request(&req).await.unwrap(), Reply::NoteEdited);
    e.until(&mut events, |_, answered| answered).await;
    assert_eq!(e.text, text);
    let refused = c.request(&e.edit(Delta::splice(0, 0, "x"))).await.unwrap();
    assert!(matches!(&refused, Reply::NoteRefused { reason } if reason.contains("256 KiB")), "{refused:?}");

    // However fast a client sends, signed edits are at least 25 ms apart.
    let started = Instant::now();
    for _ in 0..20 {
        e.n += 1;
        let req = e.edit(Delta::splice(0, 1, ""));
        assert_eq!(c.request(&req).await.unwrap(), Reply::NoteEdited);
    }
    assert!(started.elapsed() >= Duration::from_millis(19 * 25), "{:?}", started.elapsed());
}

/// Keystroke on A to the patch on B, over loopback. By hand:
/// `cargo test -p tether-daemon --release --test notes_e2e -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn note_latency() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, ctl_b) = Daemon::with_control("bravo");
    common::pair(&mut b, &mut a);
    let (mut ca, mut cb) = (client(&ctl_a).await, client(&ctl_b).await);
    let mut events_b = client(&ctl_b).await.subscribe().await.unwrap();
    let mut events_a = client(&ctl_a).await.subscribe().await.unwrap();
    let mut ea = Editor::open(&mut ca, NoteRef::SCRATCHPAD, 1).await;
    let mut eb = Editor::open(&mut cb, NoteRef::SCRATCHPAD, 2).await;
    let mut took = Vec::new();
    for k in 0..200 {
        let before = eb.text.len();
        let req = ea.edit(Delta::splice(k, 0, "k"));
        let started = Instant::now();
        ca.request(&req).await.unwrap();
        eb.until(&mut events_b, |e, _| e.text.len() > before).await;
        took.push(started.elapsed());
        ea.until(&mut events_a, |_, answered| answered).await;
        tokio::time::sleep(Duration::from_millis(30)).await; // the pace is not what is measured
    }
    took.sort();
    println!("edit on A to patch on B: p50 {:.2?}, p95 {:.2?}, p99 {:.2?}", took[100], took[190], took[197]);
}
