//! Keystroke to ranked results over 2000 apps (budget: 16 ms at p95).

use criterion::{Criterion, criterion_group, criterion_main};
use daimos_core::Item;
use daimos_core::apps::{AppAction, AppEntry};
use daimos_core::frecency::Frecency;
use daimos_core::index::AppIndex;
use daimos_core::ranker::{Candidate, Ranker};

fn apps(n: usize) -> Vec<Item> {
    const WORDS: [&str; 16] = [
        "Firefox",
        "Terminal",
        "Calculator",
        "Files",
        "Settings",
        "Editor",
        "Browser",
        "Music",
        "Photos",
        "Mail",
        "Calendar",
        "Notes",
        "Maps",
        "Video",
        "Archive",
        "Monitor",
    ];
    (0..n)
        .map(|i| {
            let mut item = Item::new(
                format!("app{i}"),
                format!("{} {} {}", WORDS[i % 16], WORDS[(i / 16) % 16], i),
            );
            item.keywords = vec![WORDS[(i * 7) % 16].to_lowercase()];
            item
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let items = apps(2000);
    let candidates: Vec<_> = items.iter().map(|item| Candidate { item, weight: 1.0 }).collect();
    let frecency = Frecency::in_memory().unwrap();
    for i in 0..50 {
        frecency.record(&format!("app{}", i * 13), 0).unwrap();
    }
    let mut ranker = Ranker::new();
    let mut group = c.benchmark_group("rank_2000_apps");
    for query in ["f", "fi", "fire", "cal ed", "zzz"] {
        group.bench_function(query, |b| {
            b.iter(|| ranker.rank(query, &candidates, &frecency, 0, 50))
        });
    }
    group.bench_function("empty", |b| b.iter(|| ranker.rank("", &candidates, &frecency, 0, 50)));
    group.finish();
}

/// Loading the stored launches at start: 10 000 rows from SQLite into memory.
fn bench_load(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frecency.db");
    {
        let frecency = Frecency::open(&path).unwrap();
        for i in 0..10_000 {
            frecency.record(&format!("app{}", i % 2000), i64::from(i)).unwrap();
        }
    }
    c.bench_function("frecency_load_10k/open", |b| b.iter(|| Frecency::open(&path).unwrap()));
}

/// Loading 2000 stored apps (name, keywords, actions, icon) into memory at start.
fn bench_index_load(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.db");
    let apps: Vec<AppEntry> = (0..2000)
        .map(|i| AppEntry {
            id: format!("linux:app{i}.desktop"),
            name: format!("Application {i}"),
            generic_name: Some("Generic".into()),
            keywords: vec!["one".into(), "two".into(), "three".into()],
            exec: format!("/usr/bin/app{i} --flag"),
            terminal: false,
            icon: Some(format!("/usr/share/icons/hicolor/64x64/apps/app{i}.png").into()),
            actions: vec![AppAction {
                id: "new".into(),
                name: "New Window".into(),
                exec: format!("/usr/bin/app{i} --new"),
            }],
        })
        .collect();
    AppIndex::open(&path).unwrap().replace("linux", &apps, 1).unwrap();
    let index = AppIndex::open(&path).unwrap();
    c.bench_function("index_load_2000/load", |b| b.iter(|| index.load().unwrap()));
}

criterion_group!(benches, bench, bench_load, bench_index_load);
criterion_main!(benches);
