//! The table of clipboard entries.
//!
//! Equal content is stored once: copying it again moves the entry to the top. Pinned entries are
//! never removed by the limit on the number of entries.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use super::{ClipboardContent, ClipboardSnapshot};

const MIGRATIONS: &[&str] = &["
    CREATE TABLE clipboard (
        id        INTEGER PRIMARY KEY,
        kind      TEXT NOT NULL,
        hash      INTEGER NOT NULL,
        text      TEXT,
        image     BLOB,
        size      INTEGER NOT NULL,
        pinned    INTEGER NOT NULL DEFAULT 0,
        copied_at INTEGER NOT NULL
    );
    CREATE INDEX clipboard_hash ON clipboard (hash);
    CREATE INDEX clipboard_copied ON clipboard (copied_at);
"];

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("clipboard database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("cannot create {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Entries kept, not counting pinned ones.
    pub max_entries: usize,
    /// Larger images are not stored.
    pub max_image_bytes: usize,
    /// Larger texts are not stored.
    pub max_text_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_entries: 500, max_image_bytes: 10 * 1024 * 1024, max_text_bytes: 1024 * 1024 }
    }
}

/// A stored entry; images are loaded with [`ClipboardHistory::image`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub text: Option<String>,
    pub is_image: bool,
    pub size: usize,
    pub pinned: bool,
    /// Seconds since the Unix epoch.
    pub copied_at: i64,
}

pub struct ClipboardHistory {
    db: Mutex<Connection>,
    limits: Mutex<Limits>,
}

impl ClipboardHistory {
    pub fn open(path: &Path, limits: Limits) -> Result<Self, HistoryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| HistoryError::Io { path: parent.to_owned(), source })?;
        }
        Self::from_connection(Connection::open(path)?, limits)
    }

    pub fn in_memory(limits: Limits) -> Result<Self, HistoryError> {
        Self::from_connection(Connection::open_in_memory()?, limits)
    }

    fn from_connection(db: Connection, limits: Limits) -> Result<Self, HistoryError> {
        let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        for (index, migration) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            db.execute_batch(migration)?;
            db.pragma_update(None, "user_version", (index + 1) as i64)?;
        }
        Ok(Self { db: Mutex::new(db), limits: Mutex::new(limits) })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_limits(&self, limits: Limits) {
        *self.limits.lock().unwrap_or_else(|e| e.into_inner()) = limits;
    }

    pub fn limits(&self) -> Limits {
        *self.limits.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stores a change. Returns the id of the entry, or `None` when it was skipped: concealed,
    /// empty, a text of only whitespace, or over the size limit.
    pub fn record(&self, snapshot: &ClipboardSnapshot) -> Result<Option<i64>, HistoryError> {
        self.record_at(snapshot, now())
    }

    fn record_at(
        &self,
        snapshot: &ClipboardSnapshot,
        now: i64,
    ) -> Result<Option<i64>, HistoryError> {
        let limits = self.limits();
        let content = &snapshot.content;
        let too_big = match content {
            ClipboardContent::Text(_) => content.len() > limits.max_text_bytes,
            ClipboardContent::Image(_) => content.len() > limits.max_image_bytes,
        };
        let blank = match content {
            ClipboardContent::Text(text) => text.trim().is_empty(),
            ClipboardContent::Image(bytes) => bytes.is_empty(),
        };
        if snapshot.concealed || blank || too_big {
            return Ok(None);
        }
        let (kind, text, image): (&str, Option<&str>, Option<&[u8]>) = match content {
            ClipboardContent::Text(text) => ("text", Some(text), None),
            ClipboardContent::Image(bytes) => ("image", None, Some(bytes)),
        };
        let hash = fnv1a(kind, text.map(str::as_bytes).or(image).unwrap_or_default());
        let db = self.db();

        let existing: Option<i64> = db
            .query_row(
                "SELECT id FROM clipboard WHERE hash = ?1 AND kind = ?2
                   AND ifnull(text, '') = ?3 AND ifnull(image, x'') = ?4",
                params![hash, kind, text.unwrap_or_default(), image.unwrap_or_default()],
                |row| row.get(0),
            )
            .optional()?;
        let id = if let Some(id) = existing {
            db.execute("UPDATE clipboard SET copied_at = ?1 WHERE id = ?2", params![now, id])?;
            id
        } else {
            db.execute(
                "INSERT INTO clipboard (kind, hash, text, image, size, copied_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![kind, hash, text, image, content.len() as i64, now],
            )?;
            db.last_insert_rowid()
        };
        db.execute(
            "DELETE FROM clipboard WHERE pinned = 0 AND id NOT IN (
                 SELECT id FROM clipboard WHERE pinned = 0
                 ORDER BY copied_at DESC, id DESC LIMIT ?1)",
            params![limits.max_entries as i64],
        )?;
        Ok(Some(id))
    }

    /// Newest first, pinned entries before the others.
    pub fn list(&self) -> Result<Vec<Entry>, HistoryError> {
        let db = self.db();
        let mut stmt = db.prepare_cached(
            "SELECT id, text, kind = 'image', size, pinned, copied_at FROM clipboard
             ORDER BY pinned DESC, copied_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Entry {
                id: row.get(0)?,
                text: row.get(1)?,
                is_image: row.get(2)?,
                size: row.get::<_, i64>(3)? as usize,
                pinned: row.get(4)?,
                copied_at: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn image(&self, id: i64) -> Result<Option<Vec<u8>>, HistoryError> {
        Ok(self
            .db()
            .query_row("SELECT image FROM clipboard WHERE id = ?1", [id], |row| row.get(0))
            .optional()?
            .flatten())
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<(), HistoryError> {
        self.db().execute("UPDATE clipboard SET pinned = ?1 WHERE id = ?2", params![pinned, id])?;
        Ok(())
    }

    pub fn remove(&self, id: i64) -> Result<(), HistoryError> {
        self.db().execute("DELETE FROM clipboard WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Removes everything that is not pinned.
    pub fn clear(&self) -> Result<(), HistoryError> {
        self.db().execute("DELETE FROM clipboard WHERE pinned = 0", [])?;
        Ok(())
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// FNV-1a over the kind and the bytes; only a filter for the lookup, equality is checked in SQL.
fn fnv1a(kind: &str, bytes: &[u8]) -> i64 {
    let hash = kind.as_bytes().iter().chain(bytes).fold(0xcbf2_9ce4_8422_2325_u64, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    hash as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> ClipboardSnapshot {
        ClipboardSnapshot { content: ClipboardContent::Text(s.into()), concealed: false }
    }

    fn image(bytes: &[u8]) -> ClipboardSnapshot {
        ClipboardSnapshot { content: ClipboardContent::Image(bytes.to_vec()), concealed: false }
    }

    fn history(max_entries: usize) -> ClipboardHistory {
        ClipboardHistory::in_memory(Limits { max_entries, ..Limits::default() }).unwrap()
    }

    fn texts(history: &ClipboardHistory) -> Vec<String> {
        history.list().unwrap().into_iter().filter_map(|e| e.text).collect()
    }

    #[test]
    fn newest_entry_comes_first() {
        let h = history(10);
        h.record_at(&text("a"), 1).unwrap();
        h.record_at(&text("b"), 2).unwrap();
        assert_eq!(texts(&h), ["b", "a"]);
    }

    #[test]
    fn copying_the_same_content_again_moves_it_to_the_top() {
        let h = history(10);
        let a = h.record_at(&text("a"), 1).unwrap();
        h.record_at(&text("b"), 2).unwrap();
        assert_eq!(h.record_at(&text("a"), 3).unwrap(), a);
        assert_eq!(texts(&h), ["a", "b"]);
    }

    #[test]
    fn equal_text_and_image_bytes_are_different_entries() {
        let h = history(10);
        h.record_at(&text("abc"), 1).unwrap();
        h.record_at(&image(b"abc"), 2).unwrap();
        assert_eq!(h.list().unwrap().len(), 2);
    }

    #[test]
    fn the_oldest_entries_beyond_the_maximum_are_removed() {
        let h = history(2);
        for (i, s) in ["a", "b", "c"].iter().enumerate() {
            h.record_at(&text(s), i as i64).unwrap();
        }
        assert_eq!(texts(&h), ["c", "b"]);
    }

    #[test]
    fn pinned_entries_survive_the_maximum_and_stay_on_top() {
        let h = history(1);
        let a = h.record_at(&text("a"), 1).unwrap().unwrap();
        h.set_pinned(a, true).unwrap();
        h.record_at(&text("b"), 2).unwrap();
        h.record_at(&text("c"), 3).unwrap();
        assert_eq!(texts(&h), ["a", "c"]);
        h.clear().unwrap();
        assert_eq!(texts(&h), ["a"]);
    }

    #[test]
    fn concealed_blank_and_oversized_content_is_skipped() {
        let h = ClipboardHistory::in_memory(Limits {
            max_image_bytes: 3,
            max_text_bytes: 4,
            ..Limits::default()
        })
        .unwrap();
        let secret = ClipboardSnapshot { concealed: true, ..text("hunter2") };
        assert_eq!(h.record(&secret).unwrap(), None);
        assert_eq!(h.record(&text("  \n")).unwrap(), None);
        assert_eq!(h.record(&text("12345")).unwrap(), None);
        assert_eq!(h.record(&image(b"1234")).unwrap(), None);
        assert!(h.record(&text("1234")).unwrap().is_some());
        assert!(h.record(&image(b"123")).unwrap().is_some());
        assert_eq!(h.list().unwrap().len(), 2);
    }

    #[test]
    fn images_round_trip() {
        let h = history(10);
        let id = h.record(&image(&[0, 1, 2, 255])).unwrap().unwrap();
        assert_eq!(h.image(id).unwrap(), Some(vec![0, 1, 2, 255]));
        let entry = &h.list().unwrap()[0];
        assert!(entry.is_image && entry.text.is_none() && entry.size == 4);
    }

    #[test]
    fn entries_are_removed_one_by_one() {
        let h = history(10);
        let a = h.record(&text("a")).unwrap().unwrap();
        h.record(&text("b")).unwrap();
        h.remove(a).unwrap();
        assert_eq!(texts(&h), ["b"]);
    }

    #[test]
    fn the_history_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clipboard.db");
        ClipboardHistory::open(&path, Limits::default()).unwrap().record(&text("kept")).unwrap();
        let again = ClipboardHistory::open(&path, Limits::default()).unwrap();
        assert_eq!(texts(&again), ["kept"]);
    }
}
