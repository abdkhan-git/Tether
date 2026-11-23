//! A paired device goes away and comes back with the same identity (a
//! restart, or a reboot): the other one reconnects by itself, and sync and
//! history carry on. `#[ignore]`d because a restart needs a persistent
//! identity, which lives in the real OS credential store; the test deletes
//! its profiles afterwards. Run by hand:
//!
//! ```text
//! cargo test -p tether-daemon --test reconnect_e2e -- --ignored --nocapture
//! ```

mod common;

use std::time::{Duration, Instant};

use common::{forget_profile, pair, unique_profile, Daemon};
use tether_ipc::{Client, Reply, Request};

/// Deletes the test profiles when dropped, even if the test fails.
struct Forget(Vec<String>);

impl Drop for Forget {
    fn drop(&mut self) {
        self.0.iter().for_each(|p| forget_profile(p));
    }
}

fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "uses the OS credential store"]
async fn a_restarted_device_is_reconnected_and_keeps_its_history() {
    let (pa, pb) = (unique_profile("re-a"), unique_profile("re-b"));
    let _forget = Forget(vec![pa.clone(), pb.clone()]); // dropped last
    let port = free_udp_port();
    let a_addr = format!("127.0.0.1:{port}");

    let mut a = Daemon::persistent("alpha", &pa, port, &[]);
    let mut b = Daemon::persistent("bravo", &pb, 0, &["--peer", &a_addr]);
    pair(&mut a, &mut b);
    let m = a.mark();
    b.send("before the restart");
    assert!(a.wait_for("<< before the restart", m).is_some(), "sync works before the restart");

    // 1. A quick restart, shorter than the 15 s idle timeout: B still holds
    //    the old connection. The restarted A answers B's next keep-alive with
    //    a stateless reset (its keys are derived from the device key), so B
    //    redials within about one keep-alive interval (5 s).
    drop(a); // hard kill: no CONNECTION_CLOSE is sent
    let mb = b.mark();
    std::thread::sleep(Duration::from_secs(6));
    let a = Daemon::persistent("alpha", &pa, port, &[]);
    let back = Instant::now();
    let line = b
        .wait_for_within("connected peer=", mb, Duration::from_millis(5), Duration::from_secs(30))
        .unwrap_or_else(|| panic!("B reconnects after a quick restart:\n{}", b.since(mb)));
    let quick = back.elapsed();
    assert!(line.contains(&a.id[..8]), "{line}");
    println!("quick restart (down 6 s): B reconnected {quick:?} after A was back");
    assert!(quick < Duration::from_secs(8), "the stateless reset should beat the 15 s idle timeout: {quick:?}");

    // 2. A long outage, longer than the idle timeout: B's connection times
    //    out, its dials fail and back off, and it reconnects when A returns.
    drop(a);
    let mb = b.mark();
    std::thread::sleep(Duration::from_secs(25));
    let mut a = Daemon::persistent("alpha", &pa, port, &[]);
    let back = Instant::now();
    b.wait_for_within("connected peer=", mb, Duration::from_millis(5), Duration::from_secs(45))
        .unwrap_or_else(|| panic!("B reconnects after a long outage:\n{}", b.since(mb)));
    let long = back.elapsed();
    let log = b.since(mb);
    let retries: Vec<&str> = log.lines().filter(|l| l.contains("backing off")).collect();
    println!("long outage (down 25 s): B reconnected {long:?} after A was back; B's backoff while A was down:");
    retries.iter().for_each(|l| println!("  {}", l.split("tetherd::net: ").last().unwrap_or(l)));
    assert!(!retries.is_empty(), "B backed off while A was down:\n{log}");

    // Sync works again in both directions.
    let m = a.mark();
    b.send("after the restart");
    assert!(a.wait_for("<< after the restart", m).is_some(), "B to A after the restart");
    let m = b.mark();
    a.send("from the restarted alpha");
    assert!(b.wait_for("<< from the restarted alpha", m).is_some(), "A to B after the restart");

    // A's history survived the hard kill (encrypted store, reloaded).
    let mut client = Client::connect(&pa).await.expect("A's control channel");
    let Reply::History(page) = client.request(&Request::History { before: None, limit: 10 }).await.unwrap() else {
        panic!("no history")
    };
    let texts: Vec<&str> = page.items.iter().map(|i| i.text.as_str()).collect();
    assert!(texts.contains(&"before the restart") && texts.contains(&"after the restart"), "{texts:?}");
}
