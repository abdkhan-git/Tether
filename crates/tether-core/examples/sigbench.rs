//! Rough cost of signing and verifying one operation on this machine.
use std::time::Instant;
use tether_core::{DeviceKey, Op, OpKind, Timestamp};

fn main() {
    let key = DeviceKey::for_test(1);
    let n = 2_000u64;
    let t = Instant::now();
    let ops: Vec<Op> = (1..=n)
        .map(|i| Op::sign(&key, i, Timestamp { wall_ms: i, counter: 0 }, OpKind::ClipboardText(format!("item {i}"))))
        .collect();
    let sign = t.elapsed() / n as u32;
    let t = Instant::now();
    assert!(ops.iter().all(Op::signature_is_valid));
    let verify = t.elapsed() / n as u32;
    println!("sign {sign:?} per op, verify {verify:?} per op");
}
