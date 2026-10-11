//! Keystroke to ranked results over 2000 apps (budget: 16 ms at p95).

use criterion::{Criterion, criterion_group, criterion_main};
use daimos_core::Item;
use daimos_core::frecency::Frecency;
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

criterion_group!(benches, bench);
criterion_main!(benches);
