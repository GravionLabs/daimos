//! User settings, stored in the SQLite table `settings` of `<config dir>/settings.db`.
//!
//! The table has one row (`id = 1`). Schema changes are embedded migrations (`PRAGMA user_version`);
//! the `settings.json` of earlier builds is imported once, when the table has no row yet.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// The default hotkey: `Option+Space` on macOS, `Alt+Space` on Linux and Windows.
pub fn default_hotkey() -> String {
    if cfg!(target_os = "macos") { "Option+Space" } else { "Alt+Space" }.to_owned()
}

/// Which color scheme the palette uses: the one of the OS, or one forced in the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSetting {
    #[default]
    System,
    Light,
    Dark,
}

/// What the user chose for one provider (apps, calculator, …). Unlisted providers are on with
/// their own weight.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(default)]
pub struct ProviderSetting {
    pub enabled: bool,
    /// Overrides the provider's own weight, which orders its results among the other providers'.
    pub weight: Option<f32>,
}

impl Default for ProviderSetting {
    fn default() -> Self {
        Self { enabled: true, weight: None }
    }
}

/// The clipboard history. It is opt-in: copied text can contain secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardSettings {
    pub enabled: bool,
    /// Entries kept besides the pinned ones.
    pub max_entries: usize,
    /// Larger images are not stored.
    pub max_image_mb: usize,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self { enabled: false, max_entries: 500, max_image_mb: 10 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The accelerator that toggles the window, for example `Alt+Space`.
    pub hotkey: String,
    pub theme: ThemeSetting,
    /// By provider id.
    pub providers: BTreeMap<String, ProviderSetting>,
    /// Short names for what to search: typing the alias (the whole query) searches the target.
    pub aliases: BTreeMap<String, String>,
    pub clipboard: ClipboardSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            theme: ThemeSetting::default(),
            providers: BTreeMap::new(),
            aliases: BTreeMap::new(),
            clipboard: ClipboardSettings::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("cannot read or write {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path} is not valid settings JSON: {source}")]
    Parse { path: PathBuf, source: serde_json::Error },
    #[error("settings database {path}: {source}")]
    Db { path: PathBuf, source: rusqlite::Error },
}

/// Embedded migrations; the position in the list is the schema version.
const MIGRATIONS: &[&str] = &[
    // 1: hotkey and theme.
    "CREATE TABLE settings (
        id     INTEGER PRIMARY KEY CHECK (id = 1),
        hotkey TEXT NOT NULL,
        theme  TEXT NOT NULL
    );",
    // 2: providers on/off with weights, and aliases (JSON objects).
    "ALTER TABLE settings ADD COLUMN providers TEXT NOT NULL DEFAULT '{}';
     ALTER TABLE settings ADD COLUMN aliases   TEXT NOT NULL DEFAULT '{}';",
    // 3: the clipboard history (a JSON object).
    "ALTER TABLE settings ADD COLUMN clipboard TEXT NOT NULL DEFAULT '{}';",
];

/// Reads and writes [`Settings`].
#[derive(Debug, Clone)]
pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self { dir: dir.as_ref().to_owned() }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("settings.db")
    }

    fn json_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    fn open(&self) -> Result<Connection, SettingsError> {
        let path = self.db_path();
        let db_error = |source| SettingsError::Db { path: path.clone(), source };
        std::fs::create_dir_all(&self.dir)
            .map_err(|source| SettingsError::Io { path: self.dir.clone(), source })?;
        let db = Connection::open(&path).map_err(db_error)?;
        migrate(&db).map_err(db_error)?;
        Ok(db)
    }

    /// No row (and no old JSON file) means defaults; fields a row cannot hold fall back to their
    /// defaults.
    pub fn load(&self) -> Result<Settings, SettingsError> {
        let db = self.open()?;
        let db_error = |source| SettingsError::Db { path: self.db_path(), source };
        let row = db
            .query_row(
                "SELECT hotkey, theme, providers, aliases, clipboard FROM settings WHERE id = 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?;
        match row {
            Some((hotkey, theme, providers, aliases, clipboard)) => Ok(Settings {
                hotkey,
                theme: serde_json::from_value(serde_json::Value::String(theme)).unwrap_or_default(),
                providers: serde_json::from_str(&providers).unwrap_or_default(),
                aliases: serde_json::from_str(&aliases).unwrap_or_default(),
                clipboard: serde_json::from_str(&clipboard).unwrap_or_default(),
            }),
            None => self.import_json(),
        }
    }

    /// The settings of an earlier build: a JSON file next to the database. It is read once and
    /// stored; a damaged file is an error rather than a silent reset.
    fn import_json(&self) -> Result<Settings, SettingsError> {
        let path = self.json_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let settings: Settings = serde_json::from_str(&text)
                    .map_err(|source| SettingsError::Parse { path, source })?;
                self.save(&settings)?;
                Ok(settings)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(source) => Err(SettingsError::Io { path, source }),
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let db = self.open()?;
        let theme =
            serde_json::to_value(settings.theme).ok().and_then(|v| v.as_str().map(str::to_owned));
        db.execute(
            "INSERT INTO settings (id, hotkey, theme, providers, aliases, clipboard)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET hotkey = ?1, theme = ?2, providers = ?3, aliases = ?4,
                                            clipboard = ?5",
            params![
                settings.hotkey,
                theme.unwrap_or_else(|| "system".into()),
                serde_json::to_string(&settings.providers).expect("providers serialize"),
                serde_json::to_string(&settings.aliases).expect("aliases serialize"),
                serde_json::to_string(&settings.clipboard).expect("clipboard settings serialize"),
            ],
        )
        .map(drop)
        .map_err(|source| SettingsError::Db { path: self.db_path(), source })
    }
}

fn migrate(db: &Connection) -> Result<(), rusqlite::Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SettingsStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn missing_database_gives_defaults() {
        let (_dir, store) = store();
        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn everything_is_read_back() {
        let (dir, store) = store();
        let nested = SettingsStore::new(dir.path().join("nested"));
        let settings = Settings {
            hotkey: "Ctrl+Shift+Space".into(),
            theme: ThemeSetting::Dark,
            providers: BTreeMap::from([
                ("calc".to_owned(), ProviderSetting { enabled: false, weight: None }),
                ("apps".to_owned(), ProviderSetting { enabled: true, weight: Some(3.5) }),
            ]),
            aliases: BTreeMap::from([("ff".to_owned(), "Firefox".to_owned())]),
            clipboard: ClipboardSettings { enabled: true, max_entries: 50, max_image_mb: 2 },
        };
        nested.save(&settings).unwrap();
        assert_eq!(nested.load().unwrap(), settings);
        // A second save replaces the row.
        nested.save(&Settings::default()).unwrap();
        assert_eq!(nested.load().unwrap(), Settings::default());
        drop(store);
    }

    #[test]
    fn the_clipboard_history_is_off_by_default() {
        let defaults = ClipboardSettings::default();
        assert!(!defaults.enabled);
        assert_eq!((defaults.max_entries, defaults.max_image_mb), (500, 10));
    }

    #[test]
    fn the_theme_defaults_to_the_system_and_survives_other_changes() {
        assert_eq!(Settings::default().theme, ThemeSetting::System);
        let (_dir, store) = store();
        store.save(&Settings { theme: ThemeSetting::Light, ..Settings::default() }).unwrap();
        let mut loaded = store.load().unwrap();
        assert_eq!(loaded.theme, ThemeSetting::Light);
        loaded.hotkey = "Ctrl+Space".into();
        store.save(&loaded).unwrap();
        assert_eq!(store.load().unwrap().theme, ThemeSetting::Light);
    }

    /// A database as the first schema version left it: no providers and aliases yet.
    fn version_one(dir: &Path) {
        let db = Connection::open(dir.join("settings.db")).unwrap();
        db.execute_batch(MIGRATIONS[0]).unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        db.execute("INSERT INTO settings (id, hotkey, theme) VALUES (1, 'Ctrl+Alt+K', 'dark')", [])
            .unwrap();
    }

    #[test]
    fn a_row_of_the_previous_schema_is_migrated_and_keeps_its_values() {
        let (dir, store) = store();
        version_one(dir.path());

        let loaded = store.load().unwrap();
        assert_eq!(loaded.hotkey, "Ctrl+Alt+K");
        assert_eq!(loaded.theme, ThemeSetting::Dark);
        assert!(loaded.providers.is_empty() && loaded.aliases.is_empty());

        let version: i64 = Connection::open(dir.path().join("settings.db"))
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        // The migrated row can be saved with the new columns.
        let mut changed = loaded;
        changed.aliases.insert("ff".into(), "Firefox".into());
        store.save(&changed).unwrap();
        assert_eq!(store.load().unwrap().aliases["ff"], "Firefox");
    }

    #[test]
    fn the_json_file_of_earlier_builds_is_imported_once() {
        let (dir, store) = store();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"hotkey":"Ctrl+Space","theme":"dark"}"#,
        )
        .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!((loaded.hotkey.as_str(), loaded.theme), ("Ctrl+Space", ThemeSetting::Dark));

        // Later changes live in the database; the old file is not read again.
        store.save(&Settings { hotkey: "Alt+K".into(), ..loaded }).unwrap();
        std::fs::write(dir.path().join("settings.json"), r#"{"hotkey":"Other"}"#).unwrap();
        assert_eq!(store.load().unwrap().hotkey, "Alt+K");
    }

    #[test]
    fn a_json_file_without_a_theme_keeps_the_other_values() {
        let (dir, store) = store();
        std::fs::write(dir.path().join("settings.json"), r#"{"hotkey":"Ctrl+Space"}"#).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!((loaded.hotkey.as_str(), loaded.theme), ("Ctrl+Space", ThemeSetting::System));
    }

    #[test]
    fn a_corrupt_json_file_is_an_error_not_a_reset() {
        let (dir, store) = store();
        std::fs::write(dir.path().join("settings.json"), "not json").unwrap();
        assert!(matches!(store.load(), Err(SettingsError::Parse { .. })));
    }

    #[test]
    fn default_hotkey_follows_the_platform() {
        if cfg!(target_os = "macos") {
            assert_eq!(default_hotkey(), "Option+Space");
        } else {
            assert_eq!(default_hotkey(), "Alt+Space");
        }
    }

    #[test]
    fn unknown_values_in_a_row_fall_back_instead_of_failing() {
        let (dir, store) = store();
        let db = store.open().unwrap();
        db.execute(
            "INSERT INTO settings (id, hotkey, theme, providers, aliases) VALUES (1, 'A', 'purple', 'oops', '[]')",
            [],
        )
        .unwrap();
        drop(db);
        let loaded = store.load().unwrap();
        assert_eq!(loaded.theme, ThemeSetting::System);
        assert!(loaded.providers.is_empty() && loaded.aliases.is_empty());
        drop(dir);
    }
}
