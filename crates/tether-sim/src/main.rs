//! `tether-sim [--seeds N] [--start S] [--devices D] [--steps K]`
//! Runs N seeds and reports failures. Re-run a failing seed with
//! `--start <seed> --seeds 1` to replay it exactly.

use std::time::Instant;

use tether_sim::{run, Config};

fn main() {
    let mut cfg = Config::default();
    let (mut seeds, mut start) = (10_000u64, 0u64);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().and_then(|v| v.parse::<u64>().ok());
        match (flag.as_str(), value) {
            ("--seeds", Some(v)) => seeds = v,
            ("--start", Some(v)) => start = v,
            ("--devices", Some(v)) if v >= 2 => cfg.devices = v as usize,
            ("--steps", Some(v)) => cfg.steps = v as usize,
            _ => {
                eprintln!("usage: tether-sim [--seeds N] [--start S] [--devices D>=2] [--steps K]");
                std::process::exit(2);
            }
        }
    }

    let started = Instant::now();
    let (mut failed, mut ops, mut faults) = (0u64, 0usize, 0usize);
    for seed in start..start + seeds {
        let report = run(seed, &cfg);
        ops += report.ops;
        faults += report.faults;
        if let Some(why) = report.failure {
            failed += 1;
            println!("FAIL seed {seed}: {why}");
        }
    }
    println!(
        "{seeds} seeds, {} devices, {} steps each: {failed} failed, {ops} ops, {faults} injected faults, {:.2?}",
        cfg.devices,
        cfg.steps,
        started.elapsed()
    );
    if failed > 0 {
        std::process::exit(1);
    }
}
