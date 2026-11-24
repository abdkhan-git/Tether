//! `tether-sim [--seeds N] [--start S] [--devices D] [--steps K] [--threads T]`
//! Runs N seeds and reports failures. Seeds are independent and each one is
//! fully deterministic, so they run in parallel across cores. Re-run a failing
//! seed with `--start <seed> --seeds 1` to replay it exactly.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tether_sim::{run, Config, Report};

fn main() {
    let mut cfg = Config::default();
    let (mut seeds, mut start) = (10_000u64, 0u64);
    let mut threads = std::thread::available_parallelism().map(|n| n.get() as u64).unwrap_or(4);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().and_then(|v| v.parse::<u64>().ok());
        match (flag.as_str(), value) {
            ("--seeds", Some(v)) => seeds = v,
            ("--start", Some(v)) => start = v,
            ("--devices", Some(v)) if v >= 2 => cfg.devices = v as usize,
            ("--steps", Some(v)) => cfg.steps = v as usize,
            ("--threads", Some(v)) if v >= 1 => threads = v,
            _ => {
                eprintln!("usage: tether-sim [--seeds N] [--start S] [--devices D>=2] [--steps K] [--threads T]");
                std::process::exit(2);
            }
        }
    }

    let started = Instant::now();
    let next = AtomicU64::new(start);
    let end = start + seeds;
    let mut reports: Vec<Report> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let seed = next.fetch_add(1, Ordering::Relaxed);
                        if seed >= end {
                            return mine;
                        }
                        mine.push(run(seed, &cfg));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("simulation thread panicked")).collect()
    });
    reports.sort_by_key(|r| r.seed);

    let mut failed = 0u64;
    for r in &reports {
        if let Some(why) = &r.failure {
            failed += 1;
            println!("FAIL seed {}: {why}", r.seed);
        }
    }
    let sum = |f: fn(&Report) -> usize| reports.iter().map(f).sum::<usize>();
    println!(
        "{seeds} seeds, {} devices, {} steps each, {threads} threads: {failed} failed, {} ops, {} pins/unpins/deletes ({} deletes), {} renames, {} sends, {} note edits, {} network faults, {} attacks, {} revocations, {:.2?}",
        cfg.devices,
        cfg.steps,
        sum(|r| r.ops),
        sum(|r| r.marks),
        sum(|r| r.deletes),
        sum(|r| r.renames),
        sum(|r| r.sends),
        sum(|r| r.note_edits),
        sum(|r| r.faults),
        sum(|r| r.attacks),
        sum(|r| r.revoked as usize),
        started.elapsed()
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
