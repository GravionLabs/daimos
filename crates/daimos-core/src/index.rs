//! The application index: a SQLite table of what the sources found, an in-memory snapshot that
//! searches read, and a service that reindexes in the background and when directories change.
//!
//! Start-up loads the table before anything else, so the first query already has results; the
//! reindex that follows replaces the snapshot in one step, so a query never sees a half-built list.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::apps::{AppAction, AppEntry, AppSource};

/// Changes in the application directories are collected for this long before one reindex.
pub const DEBOUNCE: Duration = Duration::from_secs(2);

/// Embedded migrations; the position in the list is the schema version (`PRAGMA user_version`).
const MIGRATIONS: &[&str] = &["
    CREATE TABLE apps (
        id           TEXT PRIMARY KEY,
        source       TEXT NOT NULL,
        name         TEXT NOT NULL,
        generic_name TEXT,
        keywords     TEXT NOT NULL,
        exec         TEXT NOT NULL,
        terminal     INTEGER NOT NULL,
        icon         TEXT,
        actions      TEXT NOT NULL,
        last_indexed INTEGER NOT NULL
    );
    CREATE INDEX apps_source ON apps (source);
"];

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("index database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("index data: {0}")]
    Data(#[from] serde_json::Error),
    #[error("cannot create {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
}

/// The table of applications.
pub struct AppIndex {
    db: Mutex<Connection>,
}

impl AppIndex {
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| IndexError::Io { path: parent.to_owned(), source })?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, IndexError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(db: Connection) -> Result<Self, IndexError> {
        migrate(&db)?;
        Ok(Self { db: Mutex::new(db) })
    }

    /// Every stored application, sorted by name then id.
    pub fn load(&self) -> Result<Vec<AppEntry>, IndexError> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = db.prepare_cached(
            "SELECT id, name, generic_name, keywords, exec, terminal, icon, actions
             FROM apps ORDER BY lower(name), id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;
        let mut apps = Vec::new();
        for row in rows {
            let (id, name, generic_name, keywords, exec, terminal, icon, actions) = row?;
            apps.push(AppEntry {
                id,
                name,
                generic_name,
                keywords: serde_json::from_str(&keywords)?,
                exec,
                terminal,
                icon: icon.map(PathBuf::from),
                actions: serde_json::from_str::<Vec<AppAction>>(&actions)?,
            });
        }
        Ok(apps)
    }

    /// Makes the rows of `source` exactly `apps`: new ones are added, changed ones updated,
    /// missing ones deleted, all in one transaction.
    pub fn replace(&self, source: &str, apps: &[AppEntry], now: i64) -> Result<(), IndexError> {
        let mut db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let tx = db.transaction()?;
        {
            let mut upsert = tx.prepare_cached(
                "INSERT INTO apps (id, source, name, generic_name, keywords, exec, terminal, icon, actions, last_indexed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (id) DO UPDATE SET
                    source = excluded.source, name = excluded.name, generic_name = excluded.generic_name,
                    keywords = excluded.keywords, exec = excluded.exec, terminal = excluded.terminal,
                    icon = excluded.icon, actions = excluded.actions, last_indexed = excluded.last_indexed",
            )?;
            for app in apps {
                upsert.execute(params![
                    app.id,
                    source,
                    app.name,
                    app.generic_name,
                    serde_json::to_string(&app.keywords)?,
                    app.exec,
                    app.terminal,
                    app.icon.as_ref().map(|p| p.to_string_lossy().into_owned()),
                    serde_json::to_string(&app.actions)?,
                    now,
                ])?;
            }
        }
        // Anything of this source that was not just written is gone from disk.
        tx.execute(
            "DELETE FROM apps WHERE source = ?1 AND last_indexed <> ?2",
            params![source, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn count(&self, source: &str) -> Result<usize, IndexError> {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let n: i64 =
            db.query_row("SELECT COUNT(*) FROM apps WHERE source = ?1", [source], |r| r.get(0))?;
        Ok(n as usize)
    }
}

fn migrate(db: &Connection) -> Result<(), IndexError> {
    let current: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (version, sql) in MIGRATIONS.iter().enumerate().skip(usize::try_from(current).unwrap_or(0))
    {
        let tx = db.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i64::try_from(version + 1).unwrap_or(i64::MAX))?;
        tx.commit()?;
    }
    Ok(())
}

/// Deletes cached icons nobody refers to any more: PNG files directly in `icon_dir` that are not
/// the icon of any app. Files elsewhere (the icon theme of the system) are never touched.
pub fn prune_icons(icon_dir: &Path, apps: &[AppEntry]) -> usize {
    let used: HashSet<&Path> = apps.iter().filter_map(|a| a.icon.as_deref()).collect();
    let Ok(read) = std::fs::read_dir(icon_dir) else { return 0 };
    read.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png") && !used.contains(p.as_path()))
        .filter(|p| std::fs::remove_file(p).is_ok())
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStatus {
    pub source: String,
    pub apps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStatus {
    pub apps: usize,
    pub sources: Vec<SourceStatus>,
    pub indexing: bool,
    /// Seconds since the epoch of the last finished reindex; 0 before the first one.
    pub last_indexed: i64,
}

struct Shared {
    index: AppIndex,
    sources: Vec<Arc<dyn AppSource>>,
    snapshot: RwLock<Arc<Vec<AppEntry>>>,
    indexing: AtomicBool,
    last_indexed: Mutex<i64>,
    /// One reindex at a time.
    reindex_lock: Mutex<()>,
    icon_dir: Option<PathBuf>,
}

/// Owns the index, the sources, and the background work.
pub struct IndexService {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    watcher: Mutex<Option<RecommendedWatcher>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Kept so the worker's channel stays open until `stop`.
    changes: Mutex<Option<mpsc::Sender<()>>>,
}

impl IndexService {
    /// Opens the index and loads the stored list into the snapshot. Nothing is scanned yet, so
    /// this is fast enough to run before the hotkey is registered.
    pub fn open(
        index: AppIndex,
        sources: Vec<Arc<dyn AppSource>>,
        icon_dir: Option<PathBuf>,
    ) -> Result<Self, IndexError> {
        let stored = index.load()?;
        let shared = Arc::new(Shared {
            index,
            sources,
            snapshot: RwLock::new(Arc::new(stored)),
            indexing: AtomicBool::new(false),
            last_indexed: Mutex::new(0),
            reindex_lock: Mutex::new(()),
            icon_dir,
        });
        Ok(Self {
            shared,
            stop: Arc::new(AtomicBool::new(false)),
            watcher: Mutex::new(None),
            worker: Mutex::new(None),
            changes: Mutex::new(None),
        })
    }

    /// The current list. Cheap: a shared pointer to an immutable list.
    pub fn apps(&self) -> Arc<Vec<AppEntry>> {
        Arc::clone(&self.shared.snapshot.read().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn source(&self, app_id: &str) -> Option<Arc<dyn AppSource>> {
        // Ids are `<source>:<...>`.
        let prefix = app_id.split(':').next()?;
        self.shared.sources.iter().find(|s| s.id() == prefix).cloned()
    }

    pub fn status(&self) -> IndexStatus {
        let apps = self.apps();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for app in apps.iter() {
            *counts.entry(app.id.split(':').next().unwrap_or("")).or_default() += 1;
        }
        let sources = self
            .shared
            .sources
            .iter()
            .map(|s| SourceStatus {
                source: s.id().to_owned(),
                apps: counts.get(s.id()).copied().unwrap_or(0),
            })
            .collect();
        IndexStatus {
            apps: apps.len(),
            sources,
            indexing: self.shared.indexing.load(Ordering::SeqCst),
            last_indexed: *self.shared.last_indexed.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }

    /// Scans every source now, stores the result and swaps the snapshot. Returns the new status.
    /// Concurrent calls queue up; the later one sees the newer state of the disk.
    pub fn rebuild(&self) -> Result<IndexStatus, IndexError> {
        reindex(&self.shared)?;
        Ok(self.status())
    }

    /// Reindexes in the background right away and again, after `debounce` of quiet, whenever a
    /// watched directory changes.
    pub fn start_background(&self, debounce: Duration) {
        let (tx, rx) = mpsc::channel::<()>();
        let shared = Arc::clone(&self.shared);
        let stop = Arc::clone(&self.stop);
        let worker = std::thread::Builder::new()
            .name("daimos-index".into())
            .spawn(move || {
                if let Err(error) = reindex(&shared) {
                    log::warn!("initial reindex failed: {error}");
                }
                while !stop.load(Ordering::SeqCst) {
                    // Wait for a change, then until the directories were quiet for `debounce`.
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(()) => {}
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    let mut quiet_since = Instant::now();
                    while quiet_since.elapsed() < debounce && !stop.load(Ordering::SeqCst) {
                        if rx.recv_timeout(Duration::from_millis(50)).is_ok() {
                            quiet_since = Instant::now();
                        }
                    }
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Err(error) = reindex(&shared) {
                        log::warn!("reindex after a change failed: {error}");
                    }
                }
            })
            .expect("spawn the index thread");
        *self.worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(worker);

        let notify_tx = tx.clone();
        let mut watcher =
            match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok_and(|e| !e.kind.is_access()) {
                    let _ = notify_tx.send(());
                }
            }) {
                Ok(watcher) => watcher,
                Err(error) => {
                    log::warn!("cannot watch the application directories: {error}");
                    return;
                }
            };
        for source in &self.shared.sources {
            for path in source.watch_paths() {
                if let Err(error) = watcher.watch(&path, RecursiveMode::Recursive) {
                    log::warn!("cannot watch {}: {error}", path.display());
                }
            }
        }
        *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(watcher);
        *self.changes.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    }

    /// Stops the watcher and the background thread.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.watcher.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.changes.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = worker.join();
        }
    }
}

impl Drop for IndexService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn reindex(shared: &Shared) -> Result<(), IndexError> {
    let _one_at_a_time = shared.reindex_lock.lock().unwrap_or_else(|e| e.into_inner());
    shared.indexing.store(true, Ordering::SeqCst);
    let result = (|| {
        let started = Instant::now();
        // Scan first (slow, no lock on the snapshot), then store and swap.
        let scanned: Vec<(String, Vec<AppEntry>)> =
            shared.sources.iter().map(|s| (s.id().to_owned(), s.scan())).collect();
        // `last_indexed` must differ from the previous run of the same source, so the stale-row
        // delete works even when two reindexes happen within one second.
        let now = {
            let last = *shared.last_indexed.lock().unwrap_or_else(|e| e.into_inner());
            now_secs().max(last + 1)
        };
        for (source, apps) in &scanned {
            shared.index.replace(source, apps, now)?;
        }
        let mut all: Vec<AppEntry> = scanned.into_iter().flat_map(|(_, apps)| apps).collect();
        all.sort_by(|a, b| {
            a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id))
        });
        if let Some(dir) = &shared.icon_dir {
            let pruned = prune_icons(dir, &all);
            if pruned > 0 {
                log::debug!("removed {pruned} unused cached icons");
            }
        }
        let count = all.len();
        *shared.snapshot.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(all);
        *shared.last_indexed.lock().unwrap_or_else(|e| e.into_inner()) = now;
        log::info!("indexed {count} apps in {:?}", started.elapsed());
        Ok(())
    })();
    shared.indexing.store(false, Ordering::SeqCst);
    result
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    fn app(id: &str, name: &str) -> AppEntry {
        AppEntry {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            keywords: vec![],
            exec: format!("run-{name}"),
            terminal: false,
            icon: None,
            actions: vec![],
        }
    }

    /// A source whose list the test changes; `scans` counts the calls.
    struct Fake {
        id: &'static str,
        apps: Mutex<Vec<AppEntry>>,
        scans: AtomicUsize,
        watch: Vec<PathBuf>,
    }

    impl Fake {
        fn new(id: &'static str, apps: Vec<AppEntry>) -> Arc<Self> {
            Arc::new(Self { id, apps: Mutex::new(apps), scans: AtomicUsize::new(0), watch: vec![] })
        }
    }

    impl AppSource for Fake {
        fn id(&self) -> &str {
            self.id
        }
        fn scan(&self) -> Vec<AppEntry> {
            self.scans.fetch_add(1, Ordering::SeqCst);
            self.apps.lock().unwrap().clone()
        }
        fn watch_paths(&self) -> Vec<PathBuf> {
            self.watch.clone()
        }
        fn launch(&self, _: &AppEntry, _: Option<&str>) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn names(service: &IndexService) -> Vec<String> {
        service.apps().iter().map(|a| a.name.clone()).collect()
    }

    #[test]
    fn the_schema_is_migrated_once_and_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        AppIndex::open(&path).unwrap().replace("linux", &[app("linux:a", "A")], 1).unwrap();
        let version: i64 = Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
        assert_eq!(AppIndex::open(&path).unwrap().load().unwrap().len(), 1);
    }

    #[test]
    fn every_field_survives_a_round_trip_and_the_list_is_sorted() {
        let index = AppIndex::in_memory().unwrap();
        let mut full = app("linux:b", "beta");
        full.generic_name = Some("Generic".into());
        full.keywords = vec!["x".into(), "y".into()];
        full.terminal = true;
        full.icon = Some(PathBuf::from("/icons/b.png"));
        full.actions =
            vec![AppAction { id: "new".into(), name: "New".into(), exec: "b --new".into() }];
        index.replace("linux", &[full.clone(), app("linux:a", "Alpha")], 1).unwrap();
        let loaded = index.load().unwrap();
        assert_eq!(loaded[0].name, "Alpha");
        assert_eq!(loaded[1], full);
    }

    #[test]
    fn replace_deletes_what_the_source_no_longer_has_and_keeps_other_sources() {
        let index = AppIndex::in_memory().unwrap();
        index.replace("linux", &[app("linux:a", "A"), app("linux:b", "B")], 1).unwrap();
        index.replace("other", &[app("other:x", "X")], 1).unwrap();
        index.replace("linux", &[app("linux:a", "A")], 2).unwrap();
        let ids: Vec<_> = index.load().unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(ids, ["linux:a", "other:x"]);
        assert_eq!(index.count("linux").unwrap(), 1);
    }

    #[test]
    fn a_renamed_app_keeps_its_id_so_its_frecency_survives() {
        let index = AppIndex::in_memory().unwrap();
        let frecency = crate::frecency::Frecency::in_memory().unwrap();
        index.replace("linux", &[app("linux:fx.desktop", "Firefox")], 1).unwrap();
        frecency.record("linux:fx.desktop", 100).unwrap();
        index.replace("linux", &[app("linux:fx.desktop", "Feuerfuchs")], 2).unwrap();
        let loaded = index.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Feuerfuchs");
        assert!(frecency.score(&loaded[0].id, 100) > 0.99);
    }

    #[test]
    fn start_up_serves_the_stored_list_without_scanning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.db");
        AppIndex::open(&path).unwrap().replace("fake", &[app("fake:a", "Stored")], 1).unwrap();
        let source = Fake::new("fake", vec![app("fake:a", "Fresh")]);
        let service =
            IndexService::open(AppIndex::open(&path).unwrap(), vec![source.clone()], None).unwrap();
        assert_eq!(names(&service), ["Stored"]);
        assert_eq!(source.scans.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn rebuild_swaps_the_list_and_a_removed_app_is_gone() {
        let source = Fake::new("fake", vec![app("fake:a", "A"), app("fake:b", "B")]);
        let service =
            IndexService::open(AppIndex::in_memory().unwrap(), vec![source.clone()], None).unwrap();
        let before = service.apps();
        let status = service.rebuild().unwrap();
        assert_eq!((status.apps, status.indexing), (2, false));
        assert!(before.is_empty(), "a snapshot taken earlier does not change");
        assert_eq!(names(&service), ["A", "B"]);

        // Twice within one second: the stale row must still go.
        *source.apps.lock().unwrap() = vec![app("fake:a", "A")];
        service.rebuild().unwrap();
        assert_eq!(names(&service), ["A"]);
        assert_eq!(service.shared.index.load().unwrap().len(), 1);
        assert!(service.status().last_indexed > 0);
    }

    #[test]
    fn the_status_counts_per_source() {
        let a = Fake::new("one", vec![app("one:x", "X"), app("one:y", "Y")]);
        let b = Fake::new("two", vec![app("two:z", "Z")]);
        let service = IndexService::open(AppIndex::in_memory().unwrap(), vec![a, b], None).unwrap();
        service.rebuild().unwrap();
        let status = service.status();
        assert_eq!(status.apps, 3);
        assert_eq!(
            status.sources,
            [
                SourceStatus { source: "one".into(), apps: 2 },
                SourceStatus { source: "two".into(), apps: 1 }
            ]
        );
        assert_eq!(service.source("two:z").unwrap().id(), "two");
        assert!(service.source("nope:z").is_none());
    }

    /// Lists the files of a directory as apps.
    struct Dir(PathBuf, AtomicUsize);

    impl AppSource for Dir {
        fn id(&self) -> &str {
            "dir"
        }
        fn scan(&self) -> Vec<AppEntry> {
            self.1.fetch_add(1, Ordering::SeqCst);
            let mut apps: Vec<_> = std::fs::read_dir(&self.0)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .map(|n| app(&format!("dir:{n}"), &n))
                .collect();
            apps.sort_by(|a, b| a.name.cmp(&b.name));
            apps
        }
        fn watch_paths(&self) -> Vec<PathBuf> {
            vec![self.0.clone()]
        }
        fn launch(&self, _: &AppEntry, _: Option<&str>) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn wait_until(what: &str, check: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if check() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("timed out waiting for {what}");
    }

    #[test]
    fn changes_in_a_watched_directory_trigger_one_debounced_reindex() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("first"), "").unwrap();
        let source = Arc::new(Dir(dir.path().to_owned(), AtomicUsize::new(0)));
        let service =
            IndexService::open(AppIndex::in_memory().unwrap(), vec![source.clone()], None).unwrap();
        service.start_background(Duration::from_millis(600));
        wait_until("the first index", || names(&service) == ["first"]);
        let after_start = source.1.load(Ordering::SeqCst);

        // A burst of changes, each inside the debounce window of the previous one.
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("new{i}")), "").unwrap();
            std::thread::sleep(Duration::from_millis(60));
        }
        wait_until("the reindex", || service.apps().len() == 6);
        std::thread::sleep(Duration::from_millis(1000));
        let scans = source.1.load(Ordering::SeqCst) - after_start;
        assert!((1..=3).contains(&scans), "{scans} reindexes for one burst");

        std::fs::remove_file(dir.path().join("first")).unwrap();
        wait_until("the removal", || !names(&service).contains(&"first".to_owned()));
        service.stop();
    }

    #[test]
    fn unused_cached_icons_are_removed_and_other_files_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let used = dir.path().join("used.png");
        for name in ["used.png", "unused.png", "note.txt"] {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }
        let mut with_icon = app("a", "A");
        with_icon.icon = Some(used.clone());
        assert_eq!(prune_icons(dir.path(), &[with_icon]), 1);
        assert!(used.exists() && dir.path().join("note.txt").exists());
        assert!(!dir.path().join("unused.png").exists());
    }
}
