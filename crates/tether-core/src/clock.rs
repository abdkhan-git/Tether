use std::time::{SystemTime, UNIX_EPOCH};

/// Source of physical time. Production uses [`SystemClock`]; the simulator
/// injects its own so that time (and clock skew) is controlled by the seed.
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> u64;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
    }
}
