//! A query round trip to a warm long-running Python script (budget: 30 ms at p95).
//!
//! Needs Python on the PATH (`python3`, or `python` on Windows); without it the benchmark prints a
//! note and measures nothing, and the budget check reports the missing benchmark.

use std::path::Path;
use std::process::Command;

use criterion::{Criterion, criterion_group, criterion_main};
use daimos_ext::Session;
use daimos_ext::dir::load_extension;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

fn python() -> Option<&'static str> {
    let candidates: &[&str] =
        if cfg!(windows) { &["python", "py", "python3"] } else { &["python3", "python"] };
    candidates.iter().copied().find(|name| {
        Command::new(name).arg("--version").output().is_ok_and(|out| out.status.success())
    })
}

fn bench(c: &mut Criterion) {
    let Some(python) = python() else {
        eprintln!("script_warm: no python3 or python on PATH, nothing measured");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("echo");
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/echo.py");
    std::fs::copy(fixture, dir.join("echo.py")).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        format!(
            "[extension]\nname = \"echo\"\ntitle = \"Echo\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
             [[commands]]\nname = \"search\"\ntitle = \"Search\"\n\n\
             [runtime.script]\ncommand = [{python:?}, \"echo.py\", \"loop\"]\nlong_running = true\n"
        ),
    )
    .unwrap();
    let loaded = load_extension(&dir).unwrap();
    let session = Session::new("echo", dir, loaded.manifest.runtime.script.as_ref().unwrap());
    let runtime =
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let round_trip = |text: &'static str| {
        let session = session.clone();
        runtime.block_on(async move {
            session
                .query("search", text, Default::default(), CancellationToken::new())
                .collect::<Vec<_>>()
                .await
        })
    };
    // The first query starts the process; the measurement is of the warm one.
    assert_eq!(round_trip("warm up").len(), 2);
    c.bench_function("script_warm_round_trip/query", |b| b.iter(|| round_trip("lau")));
    session.shutdown();
}

criterion_group!(benches, bench);
criterion_main!(benches);
