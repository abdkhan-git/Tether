//! Hybrid logical clock (Kulkarni et al., 2014).
//!
//! Wall-clock time alone cannot order events across devices: clocks drift and
//! can jump backwards. An HLC timestamp stays close to wall time but never goes
//! backwards on one device, and is always greater than any timestamp that
//! device has already seen from others.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp {
    pub wall_ms: u64,
    pub counter: u32,
}

#[derive(Debug, Default, Clone)]
pub struct Hlc {
    last: Timestamp,
}

impl Hlc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn last(&self) -> Timestamp {
        self.last
    }

    /// Timestamp for a local event (for example, the user copied something).
    pub fn tick(&mut self, physical_ms: u64) -> Timestamp {
        self.last = if physical_ms > self.last.wall_ms {
            Timestamp { wall_ms: physical_ms, counter: 0 }
        } else {
            Timestamp { wall_ms: self.last.wall_ms, counter: self.last.counter + 1 }
        };
        self.last
    }

    /// Merge a timestamp received from another device.
    pub fn observe(&mut self, remote: Timestamp, physical_ms: u64) -> Timestamp {
        let local = self.last;
        let wall = physical_ms.max(local.wall_ms).max(remote.wall_ms);
        let counter = if wall == local.wall_ms && wall == remote.wall_ms {
            local.counter.max(remote.counter) + 1
        } else if wall == local.wall_ms {
            local.counter + 1
        } else if wall == remote.wall_ms {
            remote.counter + 1
        } else {
            0
        };
        self.last = Timestamp { wall_ms: wall, counter };
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_is_monotonic_even_when_the_clock_goes_backwards() {
        let mut hlc = Hlc::new();
        let a = hlc.tick(1_000);
        let b = hlc.tick(400); // clock jumped back 600 ms
        let c = hlc.tick(400);
        assert!(a < b && b < c);
        assert_eq!(b.wall_ms, 1_000);
    }

    #[test]
    fn observe_moves_past_a_remote_timestamp_from_the_future() {
        let mut hlc = Hlc::new();
        hlc.tick(1_000);
        let remote = Timestamp { wall_ms: 5_000, counter: 7 };
        let merged = hlc.observe(remote, 1_001);
        assert!(merged > remote);
        assert!(hlc.tick(1_002) > merged);
    }

    #[test]
    fn physical_time_wins_once_it_catches_up() {
        let mut hlc = Hlc::new();
        hlc.observe(Timestamp { wall_ms: 5_000, counter: 3 }, 1_000);
        assert_eq!(hlc.tick(6_000), Timestamp { wall_ms: 6_000, counter: 0 });
    }
}
