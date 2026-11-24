//! Deterministic simulation of many devices syncing through the replicated
//! log while the network drops, truncates and reorders messages, devices are
//! partitioned, clocks are skewed or jump backwards, and an adversary forges,
//! tampers with and injects operations, and a device is compromised and revoked.
//!
//! One seed drives every random choice, so a failing seed replays exactly.

use std::collections::{BTreeMap, BTreeSet};

use tether_core::{DeviceId, DeviceKey, ItemId, Op, OpKind, OpLog, Timestamp};

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
    /// Probability that a sync message is lost, cut short or reordered.
    pub fault_rate: f64,
    /// Probability per step that a device's network partition state flips.
    pub partition_rate: f64,
    /// Probability per step that the adversary injects a forged operation.
    pub attack_rate: f64,
    /// Probability per run that one device is compromised and revoked.
    pub revocation_rate: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self { devices: 4, steps: 500, fault_rate: 0.2, partition_rate: 0.03, attack_rate: 0.03, revocation_rate: 0.4 }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub seed: u64,
    pub ops: usize,
    pub syncs: usize,
    pub faults: usize,
    pub blocked: usize,
    pub attacks: usize,
    /// Pins, unpins and deletes made by devices.
    pub marks: usize,
    /// Of those, deletes.
    pub deletes: usize,
    /// Items sent to one device.
    pub sends: usize,
    /// Device renames.
    pub renames: usize,
    pub revoked: bool,
    /// `None` when every invariant held.
    pub failure: Option<String>,
}

/// Text the adversary writes. No honest device may ever hold it.
const FORGED: &str = "FORGED";

struct Device {
    log: OpLog,
    skew_ms: i64,
    partitioned: bool,
    /// Operations this device has authored.
    created: u64,
    /// Set when the device is compromised and revoked: (cutoff seq).
    revoked_after: Option<u64>,
}

pub fn run(seed: u64, cfg: &Config) -> Report {
    let mut rng = Rng::new(seed);
    let mut report = Report { seed, ..Report::default() };
    let n = cfg.devices;
    let mut now: u64 = 1_700_000_000_000;

    // Device 0 creates the group and pairs with every other device.
    let mut devices: Vec<Device> = (0..n)
        .map(|i| Device {
            log: OpLog::new(DeviceKey::for_test(i as u64 + 1)),
            skew_ms: rng.below(120_001) as i64 - 60_000,
            partitioned: false,
            created: 0,
            revoked_after: None,
        })
        .collect();
    let root = devices[0].log.me();
    for i in 1..n {
        let id = devices[i].log.me();
        let t = physical(now, devices[0].skew_ms);
        devices[0]
            .log
            .append_local(OpKind::DeviceAdded { device: id, name: format!("device {i}") }, t)
            .expect("within limits");
        devices[0].created += 1;
        devices[i].log.set_root(root);
    }
    let attacker = DeviceKey::for_test(10_000 + seed);
    let revoke_at = rng.chance(cfg.revocation_rate).then(|| rng.below(cfg.steps as u64) as usize);
    // Every pin and delete a device made, kept for the oracle in `check_marks`.
    let mut marks: Vec<Op> = Vec::new();

    for step in 0..cfg.steps {
        now += 1 + rng.below(50);

        if rng.chance(cfg.partition_rate) {
            let d = rng.below(n as u64) as usize;
            devices[d].partitioned = !devices[d].partitioned;
        }
        if rng.chance(0.01) {
            let d = rng.below(n as u64) as usize;
            devices[d].skew_ms -= 5_000; // the clock jumps back five seconds
        }
        if Some(step) == revoke_at && n >= 3 {
            revoke_one(&mut devices, now, &mut rng);
            report.revoked = true;
        }
        if rng.chance(cfg.attack_rate) {
            attack(&mut devices, &attacker, now, &mut rng);
            report.attacks += 1;
        }

        let roll = rng.below(100);
        if roll < 34 {
            let d = rng.below(n as u64) as usize;
            let t = physical(now, devices[d].skew_ms);
            devices[d].log.append_local(OpKind::ClipboardText(format!("d{d}-step{step}")), t).expect("within limits");
            devices[d].created += 1;
            report.ops += 1;
        } else if roll < 40 {
            // Pin, unpin or delete something this device can see.
            let d = rng.below(n as u64) as usize;
            let history: Vec<ItemId> = devices[d].log.clipboard_history().iter().map(|op| op.id()).collect();
            if !history.is_empty() {
                let id = history[rng.below(history.len() as u64) as usize];
                let item = devices[d].log.item_ref(id).expect("an item in history can be referenced");
                let kind = match rng.below(3) {
                    0 => OpKind::Delete { item },
                    1 => OpKind::Pin { item, pinned: true },
                    _ => OpKind::Pin { item, pinned: false },
                };
                report.deletes += usize::from(matches!(kind, OpKind::Delete { .. }));
                let t = physical(now, devices[d].skew_ms);
                marks.push(devices[d].log.append_local(kind, t).expect("within limits"));
                devices[d].created += 1;
                report.marks += 1;
            }
        } else if roll < 42 {
            // Rename a member (perhaps itself).
            let d = rng.below(n as u64) as usize;
            let target = devices[rng.below(n as u64) as usize].log.me();
            let t = physical(now, devices[d].skew_ms);
            let op = devices[d]
                .log
                .append_local(OpKind::Rename { device: target, name: format!("d{d}-step{step}") }, t)
                .expect("within limits");
            marks.push(op);
            devices[d].created += 1;
            report.renames += 1;
        } else if roll < 43 {
            // Send an item to another device. Sends change no shared state:
            // the convergence and marks checks prove they leave it alone.
            let d = rng.below(n as u64) as usize;
            let to = devices[(d + 1 + rng.below(n as u64 - 1) as usize) % n].log.me();
            let first = devices[d].log.clipboard_history().first().map(|op| op.id());
            if let Some(item) = first.and_then(|id| devices[d].log.item_ref(id)) {
                let t = physical(now, devices[d].skew_ms);
                devices[d].log.append_local(OpKind::Send { item, to }, t).expect("within limits");
                devices[d].created += 1;
                report.sends += 1;
            }
        } else {
            let from = rng.below(n as u64) as usize;
            let to = (from + 1 + rng.below(n as u64 - 1) as usize) % n;
            if devices[from].partitioned || devices[to].partitioned {
                report.blocked += 1;
                continue;
            }
            report.syncs += 1;
            if !sync(&mut devices, from, to, now, cfg.fault_rate, &mut rng).0 {
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
    for _ in 0..(2 * n + 4) {
        for from in 0..n {
            for to in 0..n {
                if from != to {
                    // Page until caught up, but stop when a page makes no
                    // progress: the receiver may rightly refuse what is left.
                    loop {
                        let applied = sync(&mut devices, from, to, now, 0.0, &mut rng).1;
                        if applied == 0 || !more(&devices, from, to) {
                            break;
                        }
                    }
                }
            }
        }
    }

    report.failure = check_local_invariants(&devices)
        .or_else(|| check_convergence(&devices))
        .or_else(|| check_marks(&devices, &marks));
    report
}

fn physical(now: u64, skew_ms: i64) -> u64 {
    (now as i64 + skew_ms).max(0) as u64
}

fn more(devices: &[Device], from: usize, to: usize) -> bool {
    devices[from].log.missing_for(&devices[to].log.version_vector()).1
}

/// One-way sync `from -> to` of one page. Returns (no fault hit it, operations applied).
fn sync(devices: &mut [Device], from: usize, to: usize, now: u64, fault_rate: f64, rng: &mut Rng) -> (bool, usize) {
    let vv = devices[to].log.version_vector();
    let (mut batch, _) = devices[from].log.missing_for(&vv);
    let mut clean = true;
    if fault_rate > 0.0 && rng.chance(fault_rate) {
        clean = false;
        match rng.below(4) {
            0 => batch.clear(),                                              // lost
            1 => batch.truncate(rng.below(batch.len() as u64 + 1) as usize), // cut short
            2 if !batch.is_empty() => {
                let i = rng.below(batch.len() as u64) as usize;
                let op = batch[i].clone();
                batch.push(op); // duplicated
            }
            _ if batch.len() > 1 => {
                let i = rng.below(batch.len() as u64) as usize;
                let j = rng.below(batch.len() as u64) as usize;
                batch.swap(i, j); // reordered
            }
            _ => {}
        }
    }
    let t = physical(now, devices[to].skew_ms);
    let applied = devices[to].log.apply_batch(batch, t).applied;
    (clean, applied)
}

/// An honest device revokes another (not the root), which is then treated as
/// compromised: it keeps writing, and none of those writes may stick.
fn revoke_one(devices: &mut [Device], now: u64, rng: &mut Rng) {
    let n = devices.len();
    let target = 1 + rng.below(n as u64 - 1) as usize;
    let revoker = (0..n)
        .filter(|&i| i != target && devices[i].revoked_after.is_none())
        .nth(rng.below(n as u64 - 1) as usize % (n - 1));
    let Some(revoker) = revoker else { return };
    let target_id = devices[target].log.me();
    let cutoff = devices[revoker].log.ops_from(target_id).len() as u64;
    let t = physical(now, devices[revoker].skew_ms);
    devices[revoker]
        .log
        .append_local(OpKind::DeviceRevoked { device: target_id, after_seq: cutoff }, t)
        .expect("within limits");
    devices[revoker].created += 1;
    devices[target].revoked_after = Some(cutoff);
}

/// The adversary injects a forged, tampered or foreign operation into a device.
fn attack(devices: &mut [Device], attacker: &DeviceKey, now: u64, rng: &mut Rng) {
    let n = devices.len();
    let victim = rng.below(n as u64) as usize;
    let author = rng.below(n as u64) as usize;
    let author_id = devices[author].log.me();
    let next = devices[victim].log.ops_from(author_id).len() as u64 + 1;
    let ts = devices[victim].log.clock();
    let forged = match rng.below(6) {
        // Renames a member, signed by the attacker, claiming to be a member.
        5 => {
            let mut op = Op::sign(attacker, next, ts, OpKind::Rename { device: author_id, name: FORGED.into() });
            op.device = author_id;
            op
        }
        // Deletes a real item, signed by the attacker, claiming to be a member.
        // If it stuck, `check_marks` would find the item missing.
        4 => {
            let target = devices[victim].log.clipboard_history().first().map(|op| op.id());
            let Some(item) = target.and_then(|id| devices[victim].log.item_ref(id)) else { return };
            let mut op = Op::sign(attacker, next, ts, OpKind::Delete { item });
            op.device = author_id;
            op
        }
        // Signed by the attacker, claiming to be a member.
        0 => {
            let mut op = Op::sign(attacker, next, ts, OpKind::ClipboardText(FORGED.into()));
            op.device = author_id;
            op
        }
        // The attacker's own key, which no member ever added.
        1 => Op::sign(attacker, 1, ts, OpKind::ClipboardText(FORGED.into())),
        // A real operation with its contents changed.
        2 => match devices[author].log.ops_from(author_id).last() {
            Some(real) => {
                let mut op = real.clone();
                op.kind = OpKind::ClipboardText(FORGED.into());
                op
            }
            None => return,
        },
        // The attacker tries to add itself to the group.
        _ => {
            let mut op =
                Op::sign(attacker, next, ts, OpKind::DeviceAdded { device: attacker.id(), name: FORGED.into() });
            op.device = author_id;
            op
        }
    };
    let t = physical(now, devices[victim].skew_ms);
    devices[victim].log.apply_batch(vec![forged], t);
}

fn holds_forgery(log: &OpLog) -> bool {
    log.all_ops().any(|op| match &op.kind {
        OpKind::ClipboardText(t) => t == FORGED,
        OpKind::DeviceAdded { name, .. } => name == FORGED,
        // Forged deletes carry real item references; `check_marks` catches them.
        OpKind::Rename { name, .. } => name == FORGED,
        OpKind::DeviceRevoked { .. } | OpKind::Pin { .. } | OpKind::Delete { .. } | OpKind::Send { .. } => false,
    })
}

/// Pins and deletes, checked against an oracle built from what the devices
/// actually did (not from the log's own resolution):
/// - an item is deleted exactly when some effective delete names it (delete
///   wins; nothing else can hide an item, forged deletes included);
/// - an undeleted item is pinned exactly when its latest effective pin, by
///   (HLC time, device, seq), says so.
///
/// "Effective" means the author was not revoked, or the operation is within
/// its cutoff; and the item itself must be effective too.
fn check_marks(devices: &[Device], marks: &[Op]) -> Option<String> {
    let effective = |author: DeviceId, seq: u64| {
        devices
            .iter()
            .find(|d| d.log.me() == author)
            .is_some_and(|d| !matches!(d.revoked_after, Some(cutoff) if seq > cutoff))
    };
    let mut deleted: BTreeSet<ItemId> = BTreeSet::new();
    let mut latest: BTreeMap<ItemId, ((Timestamp, DeviceId, u64), bool)> = BTreeMap::new();
    let mut names: BTreeMap<DeviceId, ((Timestamp, DeviceId, u64), String)> = BTreeMap::new();
    for op in marks.iter().filter(|op| effective(op.device, op.seq)) {
        match &op.kind {
            OpKind::Rename { device, name } => {
                let key = (op.ts, op.device, op.seq);
                let slot = names.entry(*device).or_insert((key, name.clone()));
                if key > slot.0 {
                    *slot = (key, name.clone());
                }
            }
            OpKind::Delete { item } if effective(item.device, item.seq) => {
                deleted.insert((item.device, item.seq));
            }
            OpKind::Pin { item, pinned } if effective(item.device, item.seq) => {
                let key = (op.ts, op.device, op.seq);
                let slot = latest.entry((item.device, item.seq)).or_insert((key, *pinned));
                if key > slot.0 {
                    *slot = (key, *pinned);
                }
            }
            _ => {}
        }
    }
    let pinned: BTreeSet<ItemId> =
        latest.into_iter().filter(|(id, (_, p))| *p && !deleted.contains(id)).map(|(id, _)| id).collect();

    for d in devices.iter().filter(|d| d.revoked_after.is_none()) {
        let got = d.log.marks();
        if got.deleted != deleted {
            let (extra, missing): (Vec<_>, Vec<_>) =
                (got.deleted.difference(&deleted).collect(), deleted.difference(&got.deleted).collect());
            return Some(format!("{} deleted the wrong items: extra {extra:?}, missing {missing:?}", d.log.me()));
        }
        if got.pinned != pinned {
            return Some(format!("{} pins {:?}, expected {:?}", d.log.me(), got.pinned, pinned));
        }
        if let Some(op) = d.log.clipboard_history().iter().find(|op| deleted.contains(&op.id())) {
            return Some(format!("{} still shows deleted item {:?}", d.log.me(), op.id()));
        }
        let expected: BTreeMap<DeviceId, String> = names.iter().map(|(d, (_, n))| (*d, n.clone())).collect();
        if d.log.renames() != expected {
            return Some(format!("{} names devices {:?}, expected {expected:?}", d.log.me(), d.log.renames()));
        }
    }
    None
}

/// Invariants that must hold at every step, not only at the end.
fn check_local_invariants(devices: &[Device]) -> Option<String> {
    for d in devices {
        if holds_forgery(&d.log) {
            return Some(format!("{} accepted a forged operation", d.log.me()));
        }
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

fn check_convergence(devices: &[Device]) -> Option<String> {
    let honest: Vec<&Device> = devices.iter().filter(|d| d.revoked_after.is_none()).collect();
    // Defence in depth: `apply_remote` verifies every signature on arrival, so
    // re-verify once at the end, on one honest device (every honest device
    // holds the same operations, checked below). Every step would be too slow.
    if honest[0].log.all_ops().any(|op| !op.signature_is_valid()) {
        return Some(format!("{} holds an operation with a bad signature", honest[0].log.me()));
    }
    let first = &honest[0].log;
    let history = |log: &OpLog| log.clipboard_history().iter().map(|op| op.id()).collect::<Vec<_>>();

    for d in devices {
        let id: DeviceId = d.log.me();
        let held = first.ops_from(id).len() as u64;
        let expected = match d.revoked_after {
            None => d.created,
            Some(cutoff) => cutoff.min(d.created),
        };
        if held != expected {
            return Some(format!(
                "{id}: {} created (cutoff {:?}), {} held by {}",
                d.created,
                d.revoked_after,
                held,
                first.me()
            ));
        }
        if let Some(cutoff) = d.revoked_after {
            if !first.trust().cutoffs.contains_key(&id) || first.trust().cutoff(id) != cutoff {
                return Some(format!("{id} should be revoked after {cutoff}"));
            }
            if !d.log.is_revoked() {
                return Some(format!("{id} never learned it was revoked"));
            }
        }
    }
    for d in &honest[1..] {
        if d.log.version_vector() != first.version_vector() {
            return Some(format!("{} and {} hold different operations", d.log.me(), first.me()));
        }
        if d.log.trust() != first.trust() {
            return Some(format!("{} and {} disagree about membership", d.log.me(), first.me()));
        }
        if history(&d.log) != history(first) {
            return Some(format!("{} orders clipboard history differently from {}", d.log.me(), first.me()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_seeds_converge_under_attack() {
        // The 10,000-seed run is the CLI's job (in CI, in release mode).
        let cfg = Config::default();
        let (mut attacks, mut revocations, mut deletes, mut marks, mut renames) = (0, 0, 0, 0, 0);
        for seed in 0..300 {
            let report = run(seed, &cfg);
            assert!(report.failure.is_none(), "seed {seed} failed: {:?}", report.failure);
            attacks += report.attacks;
            revocations += report.revoked as usize;
            deletes += report.deletes;
            marks += report.marks;
            renames += report.renames;
        }
        assert!(attacks > 300 && revocations > 80, "the adversary must actually be exercised");
        assert!(deletes > 1_000 && marks > 3 * deletes / 2, "pins and deletes must actually be exercised");
        assert!(renames > 1_000, "renames must actually be exercised");
    }

    #[test]
    fn the_same_seed_gives_the_same_run() {
        let cfg = Config::default();
        let (a, b) = (run(42, &cfg), run(42, &cfg));
        assert_eq!((a.ops, a.syncs, a.faults, a.blocked, a.attacks), (b.ops, b.syncs, b.faults, b.blocked, b.attacks));
    }
}
