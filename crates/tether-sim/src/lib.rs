//! Deterministic simulation of many devices syncing through the replicated
//! log while the network drops, truncates and reorders messages, devices are
//! partitioned, and clocks are skewed or jump backwards.
//!
//! One seed drives every random choice, so a failing seed replays exactly.
//! This is the seed of stage 2: later the real daemon's networking runs on
//! simulated `Network`, `Storage` and `Clock` traits inside this harness.

use tether_core::{DeviceId, OpKind, OpLog};

/// SplitMix64: tiny, fast, and fully reproducible across platforms.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    pub fn chance(&mut self, p: f64) -> bool {
        ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) < p
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub devices: usize,
    pub steps: usize,
    /// Probability that a sync message is lost or cut short.
    pub fault_rate: f64,
    /// Probability per step that a device's network partition state flips.
    pub partition_rate: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self { devices: 4, steps: 500, fault_rate: 0.2, partition_rate: 0.03 }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub seed: u64,
    pub ops: usize,
    pub syncs: usize,
    pub faults: usize,
    pub blocked: usize,
    /// `None` when every invariant held.
    pub failure: Option<String>,
}

struct Device {
    log: OpLog,
    skew_ms: i64,
    partitioned: bool,
}

pub fn run(seed: u64, cfg: &Config) -> Report {
    let mut rng = Rng::new(seed);
    let mut report = Report { seed, ..Report::default() };
    let n = cfg.devices;
    let mut devices: Vec<Device> = (0..n)
        .map(|i| Device {
            log: OpLog::new(DeviceId::from_u128(i as u128 + 1)),
            // Clocks disagree by up to a minute either way.
            skew_ms: rng.below(120_001) as i64 - 60_000,
            partitioned: false,
        })
        .collect();
    let mut now: u64 = 1_700_000_000_000;

    for step in 0..cfg.steps {
        now += 1 + rng.below(50);

        if rng.chance(cfg.partition_rate) {
            let d = rng.below(n as u64) as usize;
            devices[d].partitioned = !devices[d].partitioned;
        }
        if rng.chance(0.01) {
            // A device's clock jumps back five seconds.
            let d = rng.below(n as u64) as usize;
            devices[d].skew_ms -= 5_000;
        }

        if rng.below(10) < 4 {
            let d = rng.below(n as u64) as usize;
            let t = physical(now, devices[d].skew_ms);
            devices[d].log.append_local(OpKind::ClipboardText(format!("d{d}-step{step}")), t);
            report.ops += 1;
        } else {
            let from = rng.below(n as u64) as usize;
            let to = (from + 1 + rng.below(n as u64 - 1) as usize) % n;
            if devices[from].partitioned || devices[to].partitioned {
                report.blocked += 1;
                continue;
            }
            report.syncs += 1;
            if !sync(&mut devices, from, to, now, cfg.fault_rate, &mut rng) {
                report.faults += 1;
            }
        }

        if let Some(failure) = check_local_invariants(&devices) {
            report.failure = Some(format!("step {step}: {failure}"));
            return report;
        }
    }

    // Heal the network and let every pair sync until nothing moves.
    for d in &mut devices {
        d.partitioned = false;
    }
    for _ in 0..n {
        for from in 0..n {
            for to in 0..n {
                if from != to {
                    sync(&mut devices, from, to, now, 0.0, &mut rng);
                }
            }
        }
    }

    report.failure = check_convergence(&devices, report.ops);
    report
}

fn physical(now: u64, skew_ms: i64) -> u64 {
    (now as i64 + skew_ms).max(0) as u64
}

/// One-way sync `from -> to`. Returns false if a fault hit the message.
fn sync(devices: &mut [Device], from: usize, to: usize, now: u64, fault_rate: f64, rng: &mut Rng) -> bool {
    let vv = devices[to].log.version_vector();
    let mut batch = devices[from].log.missing_for(&vv);
    let mut clean = true;
    if fault_rate > 0.0 && rng.chance(fault_rate) {
        clean = false;
        match rng.below(3) {
            0 => batch.clear(),                                              // lost
            1 => batch.truncate(rng.below(batch.len() as u64 + 1) as usize), // cut short
            _ if batch.len() > 1 => {
                // reordered
                let i = rng.below(batch.len() as u64) as usize;
                let j = rng.below(batch.len() as u64) as usize;
                batch.swap(i, j);
            }
            _ => {}
        }
    }
    let t = physical(now, devices[to].skew_ms);
    for op in batch {
        devices[to].log.apply_remote(op, t);
    }
    clean
}

/// Invariants that must hold at every step, not only at the end.
fn check_local_invariants(devices: &[Device]) -> Option<String> {
    for d in devices {
        for author in devices.iter().map(|x| x.log.me()) {
            let ops = d.log.ops_from(author);
            for (i, op) in ops.iter().enumerate() {
                if op.seq != i as u64 + 1 {
                    return Some(format!("{} holds {}'s ops out of sequence", d.log.me(), author));
                }
            }
            if ops.windows(2).any(|w| w[0].ts >= w[1].ts) {
                return Some(format!("{}'s timestamps went backwards on {}", author, d.log.me()));
            }
        }
    }
    None
}

fn check_convergence(devices: &[Device], total_ops: usize) -> Option<String> {
    let first = &devices[0].log;
    let history = |log: &OpLog| log.clipboard_history().iter().map(|op| op.id()).collect::<Vec<_>>();
    let expected = history(first);
    if expected.len() != total_ops {
        return Some(format!("lost operations: {} created, {} held", total_ops, expected.len()));
    }
    for d in &devices[1..] {
        if d.log.version_vector() != first.version_vector() {
            return Some(format!("{} and {} hold different operations", d.log.me(), first.me()));
        }
        if history(&d.log) != expected {
            return Some(format!("{} orders clipboard history differently from {}", d.log.me(), first.me()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thousand_seeds_converge() {
        let cfg = Config::default();
        for seed in 0..1_000 {
            let report = run(seed, &cfg);
            assert!(report.failure.is_none(), "seed {seed} failed: {:?}", report.failure);
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_run() {
        let cfg = Config::default();
        let (a, b) = (run(42, &cfg), run(42, &cfg));
        assert_eq!((a.ops, a.syncs, a.faults, a.blocked), (b.ops, b.syncs, b.faults, b.blocked));
    }
}
