//! A warm query round trip to a long-running Python script (budget: 30 ms at p95): the session is
//! started and warmed up before the measurement, so the numbers are the request, the script's
//! answer and the stream back. Skipped without Python.

#[path = "../tests/common/mod.rs"]
mod common;

use criterion::{Criterion, criterion_group, criterion_main};

fn bench(c: &mut Criterion) {
    let Some(python) = common::python() else {
        eprintln!("skipped: no python3 or python on PATH");
        return;
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let ext = common::Installed::new(python, true);
    // Starts the process and runs the first query, which pays the interpreter start.
    let warm = runtime.block_on(ext.batches("conf warm-up"));
    assert_eq!(warm.iter().map(|batch| batch.items.len()).sum::<usize>(), 3);
    c.bench_function("script_warm_round_trip/long_running", |b| {
        b.to_async(&runtime).iter(|| ext.batches("conf bench"))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
