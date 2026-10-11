//! User settings. The store is a JSON file for now; it moves into SQLite with the index (M1).

use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The accelerator that toggles the window, for example `Alt+Space`.
    pub hotkey: String,
    pub theme: ThemeSetting,
}

impl Default for Settings {
    fn default() -> Self {
        Self { hotkey: default_hotkey(), theme: ThemeSetting::default() }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("cannot read or write {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path} is not valid settings JSON: {source}")]
    Parse { path: PathBuf, source: serde_json::Error },
}

/// Reads and writes [`Settings`] in `<dir>/settings.json`.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self { path: dir.as_ref().join("settings.json") }
    }

    /// Missing file means defaults; fields missing from the file fall back to their defaults.
    pub fn load(&self) -> Result<Settings, SettingsError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|source| SettingsError::Parse { path: self.path.clone(), source }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(source) => Err(SettingsError::Io { path: self.path.clone(), source }),
        }
    }

    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let io = |source| SettingsError::Io { path: self.path.clone(), source };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let text = serde_json::to_string_pretty(settings).expect("settings serialize");
        std::fs::write(&self.path, text).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(SettingsStore::new(dir.path()).load().unwrap(), Settings::default());
    }

    #[test]
    fn saved_hotkey_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("nested"));
        store.save(&Settings { hotkey: "Ctrl+Shift+Space".into(), ..Settings::default() }).unwrap();
        assert_eq!(store.load().unwrap().hotkey, "Ctrl+Shift+Space");
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{}").unwrap();
        assert_eq!(SettingsStore::new(dir.path()).load().unwrap().hotkey, default_hotkey());
    }

    #[test]
    fn the_theme_defaults_to_the_system_and_is_stored_in_lowercase() {
        assert_eq!(Settings::default().theme, ThemeSetting::System);
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        store.save(&Settings { theme: ThemeSetting::Dark, ..Settings::default() }).unwrap();
        assert!(
            std::fs::read_to_string(dir.path().join("settings.json"))
                .unwrap()
                .contains(r#""theme": "dark""#)
        );
        assert_eq!(store.load().unwrap().theme, ThemeSetting::Dark);
    }

    #[test]
    fn a_settings_file_without_a_theme_keeps_the_other_values() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), r#"{"hotkey":"Ctrl+Space"}"#).unwrap();
        let loaded = SettingsStore::new(dir.path()).load().unwrap();
        assert_eq!((loaded.hotkey.as_str(), loaded.theme), ("Ctrl+Space", ThemeSetting::System));
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "not json").unwrap();
        assert!(matches!(SettingsStore::new(dir.path()).load(), Err(SettingsError::Parse { .. })));
    }

    #[test]
    fn default_hotkey_follows_the_platform() {
        let expected = if cfg!(target_os = "macos") { "Option+Space" } else { "Alt+Space" };
        assert_eq!(default_hotkey(), expected);
    }
}
