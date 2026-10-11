//! Frecency: how often and how recently an item was chosen.
//!
//! Every launch is a timestamp; the score of an item is the sum over its launches of
//! `0.5^(age / half-life)`. Launches live in SQLite and are loaded into memory at start, so
//! ranking never touches the disk.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, params};

/// A launch loses half of its weight after this many seconds (14 days).
pub const HALF_LIFE_SECS: f64 = 14.0 * 24.0 * 3600.0;

/// Older launches of an item add almost nothing and are pruned.
const KEEP_PER_ITEM: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum FrecencyError {
    #[error("frecency database: {0}")]
    Db(#[from] rusqlite::Error),
}

pub struct Frecency {
    db: Mutex<Connection>,
    launches: Mutex<HashMap<String, Vec<i64>>>,
}

impl Frecency {
    pub fn open(path: &Path) -> Result<Self, FrecencyError> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, FrecencyError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(db: Connection) -> Result<Self, FrecencyError> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS launches (
                item_id TEXT NOT NULL,
                at      INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS launches_item ON launches (item_id, at);",
        )?;
        let mut launches: HashMap<String, Vec<i64>> = HashMap::new();
        {
            let mut stmt = db.prepare("SELECT item_id, at FROM launches ORDER BY at")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (id, at) = row?;
                launches.entry(id).or_default().push(at);
            }
        }
        Ok(Self { db: Mutex::new(db), launches: Mutex::new(launches) })
    }

    /// Records that the user chose `item_id` at `now` (seconds since the epoch).
    pub fn record(&self, item_id: &str, now: i64) -> Result<(), FrecencyError> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        db.execute("INSERT INTO launches (item_id, at) VALUES (?1, ?2)", params![item_id, now])?;
        let mut launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
        let times = launches.entry(item_id.to_owned()).or_default();
        times.push(now);
        if times.len() > KEEP_PER_ITEM {
            let cut = times[times.len() - KEEP_PER_ITEM];
            times.drain(..times.len() - KEEP_PER_ITEM);
            db.execute(
                "DELETE FROM launches WHERE item_id = ?1 AND at < ?2",
                params![item_id, cut],
            )?;
        }
        Ok(())
    }

    /// The decayed launch count of one item at `now`; 0 when it was never chosen.
    pub fn score(&self, item_id: &str, now: i64) -> f64 {
        let launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
        launches.get(item_id).map_or(0.0, |times| decayed_sum(times, now))
    }

    /// All items with a score above zero, most frecent first, ties by id.
    pub fn top(&self, now: i64, limit: usize) -> Vec<(String, f64)> {
        let launches = self.launches.lock().unwrap_or_else(|e| e.into_inner());
        let mut all: Vec<_> =
            launches.iter().map(|(id, t)| (id.clone(), decayed_sum(t, now))).collect();
        all.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        all.truncate(limit);
        all
    }
}

fn decayed_sum(times: &[i64], now: i64) -> f64 {
    times
        .iter()
        .map(|&t| {
            let age = (now - t).max(0) as f64;
            0.5_f64.powf(age / HALF_LIFE_SECS)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 3600;

    #[test]
    fn a_launch_loses_half_its_weight_after_fourteen_days() {
        let f = Frecency::in_memory().unwrap();
        f.record("a", 0).unwrap();
        assert!((f.score("a", 0) - 1.0).abs() < 1e-9);
        assert!((f.score("a", 14 * DAY) - 0.5).abs() < 1e-9);
        assert!((f.score("a", 28 * DAY) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn scores_add_up_and_unknown_items_have_none() {
        let f = Frecency::in_memory().unwrap();
        for _ in 0..3 {
            f.record("a", 100).unwrap();
        }
        assert!((f.score("a", 100) - 3.0).abs() < 1e-9);
        assert_eq!(f.score("b", 100), 0.0);
    }

    #[test]
    fn launches_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frecency.db");
        {
            let f = Frecency::open(&path).unwrap();
            f.record("a", 10).unwrap();
            f.record("a", 20).unwrap();
        }
        let f = Frecency::open(&path).unwrap();
        assert!((f.score("a", 20) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn top_orders_by_score_then_id() {
        let f = Frecency::in_memory().unwrap();
        f.record("b", 0).unwrap();
        f.record("a", 0).unwrap();
        f.record("c", 0).unwrap();
        f.record("c", 0).unwrap();
        let ids: Vec<_> = f.top(0, 10).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["c", "a", "b"]);
        assert_eq!(f.top(0, 1).len(), 1);
    }

    #[test]
    fn old_launches_are_pruned_per_item() {
        let f = Frecency::in_memory().unwrap();
        for i in 0..(KEEP_PER_ITEM as i64 + 10) {
            f.record("a", i).unwrap();
        }
        let n: i64 =
            f.db.lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM launches", [], |r| r.get(0))
                .unwrap();
        assert_eq!(n as usize, KEEP_PER_ITEM);
    }
}
