//! When to dial a peer again.
//!
//! Pure scheduling, no I/O and no clock: callers pass the current time (any
//! monotonic milliseconds) and a random number, so every decision can be
//! tested without sleeping and replayed exactly.
//!
//! - A target the caller newly wants is due at once.
//! - After a failed dial, the next one waits an exponentially growing,
//!   capped, jittered delay ([`Policy::delay_ms`]).
//! - A connection that lasted at least [`Policy::stable_ms`] resets the
//!   backoff and is redialled at once when it ends. One that ends sooner
//!   counts as a failure, so a peer that accepts and then drops us cannot
//!   pull us into a tight loop.
//! - A target is never handed out again while a dial or session for it is
//!   still in flight.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// Ceiling of the first retry delay.
    pub base_ms: u64,
    /// Largest ceiling.
    pub cap_ms: u64,
    /// A session at least this long counts as a success.
    pub stable_ms: u64,
}

impl Default for Policy {
    fn default() -> Self {
        Self { base_ms: 1_000, cap_ms: 30_000, stable_ms: 30_000 }
    }
}

impl Policy {
    /// The ceiling after `failures` consecutive failures: `base · 2^(failures−1)`, capped.
    pub fn ceiling_ms(&self, failures: u32) -> u64 {
        if failures == 0 {
            return 0;
        }
        let doublings = (failures - 1).min(63);
        self.base_ms.saturating_mul(1u64 << doublings).min(self.cap_ms)
    }

    /// The wait before the next dial: uniform in `[ceiling/2, ceiling]`
    /// ("equal jitter"). Devices that lost the network together spread out
    /// instead of retrying in lockstep, and no retry comes sooner than half
    /// the ceiling.
    pub fn delay_ms(&self, failures: u32, random: u64) -> u64 {
        let ceiling = self.ceiling_ms(failures);
        let half = ceiling / 2;
        half + random % (ceiling - half + 1)
    }
}

/// How a dial ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The connection was never established.
    Failed,
    /// A session ran and then ended after `lasted_ms`.
    Ended { lasted_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Waiting { due_at: u64 },
    InFlight,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    state: State,
}

/// The redial schedule for a set of targets (device ids, addresses, ...).
#[derive(Debug, Clone)]
pub struct Redial<K> {
    policy: Policy,
    entries: HashMap<K, Entry>,
}

impl<K: Eq + Hash + Clone> Redial<K> {
    pub fn new(policy: Policy) -> Self {
        Self { policy, entries: HashMap::new() }
    }

    /// Say which targets should be connected now. New ones are due at once;
    /// waiting ones no longer wanted are forgotten (in-flight ones are kept
    /// until they report back).
    pub fn want(&mut self, wanted: impl IntoIterator<Item = K>, now: u64) {
        let wanted: HashSet<K> = wanted.into_iter().collect();
        self.entries.retain(|k, e| e.state == State::InFlight || wanted.contains(k));
        for k in wanted {
            self.entries.entry(k).or_insert(Entry { failures: 0, state: State::Waiting { due_at: now } });
        }
    }

    /// Targets due at `now`. Each is marked in flight and will not be
    /// returned again until reported with [`Redial::finished`].
    pub fn due(&mut self, now: u64) -> Vec<K> {
        let mut out = Vec::new();
        for (k, e) in &mut self.entries {
            if matches!(e.state, State::Waiting { due_at } if due_at <= now) {
                e.state = State::InFlight;
                out.push(k.clone());
            }
        }
        out
    }

    /// Report how a dial handed out by [`Redial::due`] ended.
    pub fn finished(&mut self, key: &K, outcome: Outcome, now: u64, random: u64) {
        let e = self.entries.entry(key.clone()).or_insert(Entry { failures: 0, state: State::InFlight });
        match outcome {
            Outcome::Ended { lasted_ms } if lasted_ms >= self.policy.stable_ms => {
                e.failures = 0;
                e.state = State::Waiting { due_at: now };
            }
            _ => {
                e.failures = e.failures.saturating_add(1);
                let delay = self.policy.delay_ms(e.failures, random);
                e.state = State::Waiting { due_at: now.saturating_add(delay) };
            }
        }
    }

    /// The earliest time a waiting target becomes due.
    pub fn next_due(&self) -> Option<u64> {
        self.entries
            .values()
            .filter_map(|e| match e.state {
                State::Waiting { due_at } => Some(due_at),
                State::InFlight => None,
            })
            .min()
    }

    /// When `key` is next due, if it is waiting (not in flight).
    pub fn due_at(&self, key: &K) -> Option<u64> {
        match self.entries.get(key)?.state {
            State::Waiting { due_at } => Some(due_at),
            State::InFlight => None,
        }
    }

    /// Consecutive failures for `key`, if it is scheduled.
    pub fn failures(&self, key: &K) -> Option<u32> {
        self.entries.get(key).map(|e| e.failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const P: Policy = Policy { base_ms: 1_000, cap_ms: 30_000, stable_ms: 30_000 };

    #[test]
    fn ceilings_double_then_cap() {
        let ceilings: Vec<u64> = (0..8).map(|f| P.ceiling_ms(f)).collect();
        assert_eq!(ceilings, [0, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000]);
        assert_eq!(P.ceiling_ms(u32::MAX), 30_000, "no overflow");
    }

    #[test]
    fn jitter_spreads_retries_out() {
        let delays: HashSet<u64> =
            (0..1_000u64).map(|r| P.delay_ms(5, r.wrapping_mul(0x9E37_79B9_7F4A_7C15))).collect();
        assert!(delays.len() > 500, "only {} distinct delays", delays.len());
        assert!(delays.iter().all(|d| (8_000..=16_000).contains(d)));
    }

    #[test]
    fn a_new_target_is_due_at_once_and_handed_out_once() {
        let mut r = Redial::new(P);
        r.want(["a"], 100);
        assert_eq!(r.next_due(), Some(100));
        assert_eq!(r.due(100), ["a"]);
        assert!(r.due(1_000_000).is_empty(), "in flight: never handed out twice");
        assert_eq!(r.next_due(), None);
    }

    #[test]
    fn failures_back_off() {
        let mut r = Redial::new(P);
        r.want(["a"], 0);
        let mut now = 0;
        for failures in 1..=7u32 {
            assert_eq!(r.due(now), ["a"]);
            r.finished(&"a", Outcome::Failed, now, 0); // random 0: the lower bound
            let wait = P.ceiling_ms(failures) / 2;
            assert_eq!(r.next_due(), Some(now + wait));
            assert!(r.due(now + wait - 1).is_empty(), "not before its time");
            now += wait;
        }
        assert_eq!(r.failures(&"a"), Some(7));
    }

    #[test]
    fn a_stable_session_resets_the_backoff_and_redials_at_once() {
        let mut r = Redial::new(P);
        r.want(["a"], 0);
        for _ in 0..4 {
            let at = r.next_due().unwrap();
            r.due(at);
            r.finished(&"a", Outcome::Failed, at, 0);
        }
        let at = r.next_due().unwrap();
        r.due(at);
        r.finished(&"a", Outcome::Ended { lasted_ms: 60_000 }, at + 60_000, 0);
        assert_eq!((r.failures(&"a"), r.next_due()), (Some(0), Some(at + 60_000)));
    }

    #[test]
    fn a_session_that_drops_quickly_counts_as_a_failure() {
        let mut r = Redial::new(P);
        r.want(["flappy"], 0);
        let mut now = 0;
        for failures in 1..=5 {
            now = r.next_due().unwrap();
            r.due(now);
            r.finished(&"flappy", Outcome::Ended { lasted_ms: 200 }, now + 200, 0);
            assert_eq!(r.failures(&"flappy"), Some(failures));
        }
        assert!(r.next_due().unwrap() >= now + 200 + P.ceiling_ms(5) / 2);
    }

    #[test]
    fn unwanted_waiting_targets_are_forgotten_but_in_flight_ones_report_back() {
        let mut r = Redial::new(P);
        r.want(["a", "b"], 0);
        let mut first = r.due(0);
        first.sort();
        assert_eq!(first, ["a", "b"]);
        r.finished(&"b", Outcome::Failed, 0, 0);
        r.want(["c"], 10); // a is in flight, b waiting: b goes, a stays
        assert_eq!(r.failures(&"b"), None);
        assert_eq!(r.failures(&"a"), Some(0));
        r.finished(&"a", Outcome::Failed, 20, 0);
        r.want(["c"], 30);
        assert_eq!(r.failures(&"a"), None, "forgotten once it is waiting and unwanted");
        assert_eq!(r.due(30), ["c"]);
    }

    proptest! {
        #[test]
        fn delays_stay_within_their_bounds(failures in 1u32..200, random: u64, base in 1u64..10_000, cap in 1u64..1_000_000) {
            let p = Policy { base_ms: base, cap_ms: cap, stable_ms: 1 };
            let ceiling = p.ceiling_ms(failures);
            let d = p.delay_ms(failures, random);
            prop_assert!(ceiling <= cap);
            prop_assert!(d >= ceiling / 2 && d <= ceiling);
        }

        /// Whatever the order of wants, dials and outcomes, a target is never
        /// handed out while a dial or session for it is still in flight.
        #[test]
        fn a_target_is_never_dialled_twice_at_once(steps in proptest::collection::vec((0u8..4, 0u8..3, any::<u64>()), 1..200)) {
            let mut r = Redial::new(P);
            let mut in_flight: HashSet<u8> = HashSet::new();
            let mut now = 0u64;
            for (action, key, random) in steps {
                now += random % 5_000;
                match action {
                    0 => r.want([0u8, 1, 2].into_iter().filter(|k| *k != key), now),
                    1 => for k in r.due(now) {
                        prop_assert!(in_flight.insert(k), "dialled {} twice at once", k);
                    },
                    2 if in_flight.remove(&key) => r.finished(&key, Outcome::Failed, now, random),
                    3 if in_flight.remove(&key) => r.finished(&key, Outcome::Ended { lasted_ms: random % 60_000 }, now, random),
                    _ => {}
                }
            }
        }
    }
}
