//! End-to-end tests of Pulse: real `tetherd` processes beating to each other
//! over loopback QUIC, read through the control channel as the app does.
//! Threat model: docs/SECURITY.md, "Pulse".

mod common;

use std::time::{Duration, Instant};

use common::{Control, Daemon};
use tether_core::pulse::Liveness;
use tether_core::DeviceId;
use tether_ipc::{Client, Device, Reply, Request};

async fn client(c: &Control) -> Client {
    Client::connect_to(&c.endpoint, &c.token).await.expect("authenticated client")
}

async fn device(c: &mut Client, id: DeviceId) -> Option<Device> {
    match c.request(&Request::Devices).await.unwrap() {
        Reply::Devices(list) => list.into_iter().find(|d| d.id == id),
        other => panic!("unexpected {other:?}"),
    }
}

/// Wait up to `limit` for `f` to hold of `id` as `c` sees it.
async fn wait_for(c: &mut Client, id: DeviceId, limit: Duration, f: impl Fn(&Device) -> bool) -> Option<Device> {
    let end = Instant::now() + limit;
    while Instant::now() < end {
        if let Some(d) = device(c, id).await.filter(|d| f(d)) {
            return Some(d);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn paired_devices_see_each_others_health_and_notice_one_that_stops() {
    let (mut a, ctl_a) = Daemon::with_control("alpha");
    let (mut b, _ctl_b) = Daemon::with_control("bravo");
    common::pair(&mut b, &mut a);
    let mut ca = client(&ctl_a).await;
    let b_id: DeviceId = b.id.parse().unwrap();
    let a_id: DeviceId = a.id.parse().unwrap();

    // Each samples its own health, and hears the other's in its beats.
    let me = wait_for(&mut ca, a_id, Duration::from_secs(10), |d| d.pulse.as_ref().is_some_and(|p| p.health.is_some()))
        .await;
    let mine = me.expect("this device's own health").pulse.unwrap().health.unwrap();
    assert!(mine.mem_total > 0 && !mine.os.is_empty() && mine.within_limits(), "{mine:?}");
    assert_eq!(ca.request(&Request::Watch).await.unwrap(), Reply::Watching);
    let peer = wait_for(&mut ca, b_id, Duration::from_secs(15), |d| {
        d.pulse.as_ref().is_some_and(|p| p.health.is_some() && p.liveness == Some(Liveness::Live))
    })
    .await
    .expect("B's health and liveness reach A");
    let pulse = peer.pulse.unwrap();
    assert!(pulse.heard_ms_ago.unwrap() < 5_000);
    assert!(pulse.health.unwrap().mem_total > 0);

    // B is killed outright (as a laptop lid closing would leave it): A's
    // detector calls it down before the 15 s QUIC idle timeout would.
    let stopped = Instant::now();
    drop(b);
    let down = wait_for(&mut ca, b_id, Duration::from_secs(14), |d| {
        d.pulse.as_ref().is_some_and(|p| p.liveness == Some(Liveness::Down))
    })
    .await;
    let took = stopped.elapsed();
    let down = down.unwrap_or_else(|| panic!("not called down within 14 s:\n{}", a.since(0)));
    assert!(took < Duration::from_secs(13), "{took:?}");
    println!("a killed peer was called down after {took:.1?}");
    println!("B can be woken: {}", down.pulse.unwrap().wakeable);

    // Nothing Pulse carries reaches the log.
    let Reply::Status(_) = ca.request(&Request::Status).await.unwrap() else { panic!() };
    assert!(!a.since(0).to_lowercase().contains("cpu"), "health values are not logged");
    let _ = &mut a;
}
