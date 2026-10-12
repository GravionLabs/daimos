//! The values of the preferences an extension declares.
//!
//! `text`, `number` and `checkbox` values are stored in SQLite (through [`SettingsStore`]);
//! `password` values only ever go to the [`SecretStore`]. A password is not logged, not part of a
//! `Debug` output of this module and not listed by `ext list`: [`Preferences::status`] reports only
//! whether one is set.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use daimos_core::settings::{SettingsError, SettingsStore};

use crate::manifest::{Manifest, Preference, PreferenceKind};
use crate::secrets::{SecretError, SecretStore};

/// Why a preference could not be read or written.
#[derive(Debug)]
pub enum PreferenceError {
    /// The extension has no preference of that name.
    Unknown(String),
    /// The value does not fit the type (`number` or `checkbox`).
    Invalid { name: String, message: String },
    /// Required preferences without a value, by name.
    Missing(Vec<String>),
    /// The OS keyring failed or is not there.
    Secret(SecretError),
    /// The database failed.
    Database(SettingsError),
}

impl fmt::Display for PreferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "there is no preference `{name}`"),
            Self::Invalid { name, message } => write!(f, "preference `{name}`: {message}"),
            Self::Missing(names) => {
                write!(f, "set the required preference(s) in the settings: {}", names.join(", "))
            }
            Self::Secret(error) => error.fmt(f),
            Self::Database(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PreferenceError {}

impl From<SecretError> for PreferenceError {
    fn from(error: SecretError) -> Self {
        Self::Secret(error)
    }
}

impl From<SettingsError> for PreferenceError {
    fn from(error: SettingsError) -> Self {
        Self::Database(error)
    }
}

/// Whether a preference has a value, without the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreferenceStatus {
    pub name: String,
    pub is_set: bool,
}

/// Reads and writes preference values, text in SQLite and passwords in the keyring.
#[derive(Clone)]
pub struct Preferences {
    settings: SettingsStore,
    secrets: Arc<dyn SecretStore>,
}

impl fmt::Debug for Preferences {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Preferences").finish_non_exhaustive()
    }
}

/// `true` or `false`, the only values of a checkbox.
fn check_value(preference: &Preference, value: &str) -> Result<(), PreferenceError> {
    let invalid = |message: &str| PreferenceError::Invalid {
        name: preference.name.clone(),
        message: message.into(),
    };
    match preference.kind {
        PreferenceKind::Number if !value.trim().parse::<f64>().is_ok_and(f64::is_finite) => {
            Err(invalid("must be a number"))
        }
        PreferenceKind::Checkbox if value != "true" && value != "false" => {
            Err(invalid("must be `true` or `false`"))
        }
        _ => Ok(()),
    }
}

impl Preferences {
    pub fn new(settings: SettingsStore, secrets: Arc<dyn SecretStore>) -> Self {
        Self { settings, secrets }
    }

    fn find<'a>(manifest: &'a Manifest, name: &str) -> Result<&'a Preference, PreferenceError> {
        manifest
            .preferences
            .iter()
            .find(|preference| preference.name == name)
            .ok_or_else(|| PreferenceError::Unknown(name.to_owned()))
    }

    /// Stores the value of the preference `name` of the extension in `manifest`.
    pub fn set(&self, manifest: &Manifest, name: &str, value: &str) -> Result<(), PreferenceError> {
        let preference = Self::find(manifest, name)?;
        check_value(preference, value)?;
        let extension = &manifest.extension.name;
        match preference.kind {
            PreferenceKind::Password => self.secrets.set(extension, name, value)?,
            _ => self.settings.set_preference(extension, name, value)?,
        }
        Ok(())
    }

    /// Removes the value of the preference `name`.
    pub fn clear(&self, manifest: &Manifest, name: &str) -> Result<(), PreferenceError> {
        let preference = Self::find(manifest, name)?;
        let extension = &manifest.extension.name;
        match preference.kind {
            PreferenceKind::Password => self.secrets.delete(extension, name)?,
            _ => self.settings.clear_preference(extension, name)?,
        }
        Ok(())
    }

    /// Which preferences of the extension have a value. A keyring that cannot be reached counts
    /// its passwords as not set.
    pub fn status(&self, manifest: &Manifest) -> Result<Vec<PreferenceStatus>, PreferenceError> {
        let extension = &manifest.extension.name;
        let stored = self.settings.preference_values(extension)?;
        Ok(manifest
            .preferences
            .iter()
            .map(|preference| {
                let is_set = match preference.kind {
                    PreferenceKind::Password => {
                        matches!(self.secrets.get(extension, &preference.name), Ok(Some(_)))
                    }
                    _ => stored.contains_key(&preference.name),
                };
                PreferenceStatus { name: preference.name.clone(), is_set }
            })
            .collect())
    }

    /// The `preferences` of a query: every preference that has a value, passwords read from the
    /// keyring now. A required preference without a value is an error naming all of them; a
    /// password that cannot be read because there is no keyring is an error when it is required
    /// and left out otherwise.
    pub fn resolve(
        &self,
        manifest: &Manifest,
    ) -> Result<BTreeMap<String, String>, PreferenceError> {
        let extension = &manifest.extension.name;
        let mut values = self.settings.preference_values(extension)?;
        // Only the declared preferences count; a leftover row of an older version does not.
        values.retain(|name, _| {
            manifest
                .preferences
                .iter()
                .any(|p| p.name == *name && p.kind != PreferenceKind::Password)
        });
        let mut missing = Vec::new();
        for preference in &manifest.preferences {
            if preference.kind == PreferenceKind::Password {
                match self.secrets.get(extension, &preference.name) {
                    Ok(Some(value)) => {
                        values.insert(preference.name.clone(), value);
                    }
                    Ok(None) => {}
                    Err(error) if preference.required => return Err(error.into()),
                    Err(error) => log::warn!(
                        "password preference `{}` of {extension} is left out: {error}",
                        preference.name
                    ),
                }
            }
            if preference.required && !values.contains_key(&preference.name) {
                missing.push(preference.name.clone());
            }
        }
        if missing.is_empty() { Ok(values) } else { Err(PreferenceError::Missing(missing)) }
    }

    /// Removes everything stored for an extension that is deleted: its SQLite values and the
    /// secrets of the password preferences of `manifest`.
    pub fn remove_extension(&self, manifest: &Manifest) -> Result<(), PreferenceError> {
        let extension = &manifest.extension.name;
        self.settings.clear_extension_preferences(extension)?;
        for preference in &manifest.preferences {
            if preference.kind == PreferenceKind::Password {
                self.secrets.delete(extension, &preference.name)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::secrets::MockSecretStore;

    use super::*;

    const MANIFEST: &str = r#"
[extension]
name = "gh"
title = "GH"
version = "1.0.0"
runtime = "script"

[[commands]]
name = "go"
title = "Go"

[runtime.script]
command = ["run.sh"]

[[preferences]]
name = "token"
title = "Token"
type = "password"
required = true

[[preferences]]
name = "user"
title = "User"
type = "text"

[[preferences]]
name = "limit"
title = "Limit"
type = "number"

[[preferences]]
name = "draft"
title = "Drafts"
type = "checkbox"
"#;

    struct Setup {
        dir: tempfile::TempDir,
        secrets: Arc<MockSecretStore>,
        preferences: Preferences,
        manifest: Manifest,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MockSecretStore::new());
        let preferences = Preferences::new(SettingsStore::new(dir.path()), secrets.clone());
        let manifest = Manifest::parse(MANIFEST, Path::new("e.toml")).unwrap();
        Setup { dir, secrets, preferences, manifest }
    }

    #[test]
    fn a_password_is_stored_and_read_under_the_extensions_name() {
        let s = setup();
        s.preferences.set(&s.manifest, "token", "s3cret").unwrap();
        assert_eq!(s.secrets.users(), ["gh/token"]);
        assert_eq!(s.secrets.get("gh", "token").unwrap().as_deref(), Some("s3cret"));
        assert_eq!(s.preferences.resolve(&s.manifest).unwrap()["token"], "s3cret");
    }

    #[test]
    fn sqlite_never_holds_a_password_value() {
        let s = setup();
        s.preferences.set(&s.manifest, "token", "s3cret-value").unwrap();
        s.preferences.set(&s.manifest, "user", "ada").unwrap();
        // The database holds the text value and nothing of the password, not even its name.
        let stored = SettingsStore::new(s.dir.path()).preference_values("gh").unwrap();
        assert_eq!(stored.keys().collect::<Vec<_>>(), ["user"]);
        let bytes = std::fs::read(s.dir.path().join("settings.db")).unwrap();
        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(!contains(b"s3cret-value"));
        assert!(contains(b"ada"));
        for entry in std::fs::read_dir(s.dir.path()).unwrap() {
            let content = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(!content.windows(12).any(|w| w == b"s3cret-value"));
        }
    }

    #[test]
    fn text_number_and_checkbox_values_are_resolved() {
        let s = setup();
        s.preferences.set(&s.manifest, "token", "t").unwrap();
        s.preferences.set(&s.manifest, "user", "ada").unwrap();
        s.preferences.set(&s.manifest, "limit", " 12.5 ").unwrap();
        s.preferences.set(&s.manifest, "draft", "true").unwrap();
        let values = s.preferences.resolve(&s.manifest).unwrap();
        assert_eq!(
            values,
            BTreeMap::from([
                ("draft".to_owned(), "true".to_owned()),
                ("limit".to_owned(), " 12.5 ".to_owned()),
                ("token".to_owned(), "t".to_owned()),
                ("user".to_owned(), "ada".to_owned()),
            ])
        );
    }

    #[test]
    fn a_required_preference_without_a_value_is_reported_by_name() {
        let s = setup();
        let error = s.preferences.resolve(&s.manifest).unwrap_err();
        assert!(matches!(&error, PreferenceError::Missing(names) if names == &["token"]));
        assert!(error.to_string().contains("token"), "{error}");
        s.preferences.set(&s.manifest, "token", "t").unwrap();
        assert!(s.preferences.resolve(&s.manifest).is_ok());
        s.preferences.clear(&s.manifest, "token").unwrap();
        assert!(matches!(s.preferences.resolve(&s.manifest), Err(PreferenceError::Missing(_))));
    }

    #[test]
    fn invalid_values_and_unknown_names_are_refused() {
        let s = setup();
        for (name, value) in
            [("limit", "many"), ("limit", "NaN"), ("limit", "inf"), ("draft", "yes"), ("draft", "")]
        {
            let error = s.preferences.set(&s.manifest, name, value).unwrap_err();
            assert!(matches!(error, PreferenceError::Invalid { .. }), "{name}={value}");
        }
        assert!(matches!(
            s.preferences.set(&s.manifest, "nope", "x"),
            Err(PreferenceError::Unknown(_))
        ));
        assert!(matches!(
            s.preferences.clear(&s.manifest, "nope"),
            Err(PreferenceError::Unknown(_))
        ));
        assert!(s.secrets.users().is_empty());
    }

    #[test]
    fn status_says_whether_a_value_is_set_and_never_shows_it() {
        let s = setup();
        s.preferences.set(&s.manifest, "token", "s3cret").unwrap();
        let status = s.preferences.status(&s.manifest).unwrap();
        assert_eq!(
            status.iter().map(|p| (p.name.as_str(), p.is_set)).collect::<Vec<_>>(),
            [("token", true), ("user", false), ("limit", false), ("draft", false)]
        );
        assert!(!format!("{status:?}{:?}", s.preferences).contains("s3cret"));
    }

    #[test]
    fn a_missing_keyring_is_a_typed_error_for_a_required_password() {
        let dir = tempfile::tempdir().unwrap();
        let preferences = Preferences::new(
            SettingsStore::new(dir.path()),
            Arc::new(MockSecretStore::unavailable()),
        );
        let manifest = Manifest::parse(MANIFEST, Path::new("e.toml")).unwrap();
        let error = preferences.set(&manifest, "token", "x").unwrap_err();
        assert!(matches!(error, PreferenceError::Secret(SecretError::Unavailable(_))));
        assert!(error.to_string().contains("OS keyring is not available"), "{error}");
        assert!(matches!(
            preferences.resolve(&manifest),
            Err(PreferenceError::Secret(SecretError::Unavailable(_)))
        ));
        // Status degrades to "not set"; text preferences still work.
        assert!(!preferences.status(&manifest).unwrap()[0].is_set);
        preferences.set(&manifest, "user", "ada").unwrap();
    }

    #[test]
    fn an_optional_password_without_a_keyring_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let preferences = Preferences::new(
            SettingsStore::new(dir.path()),
            Arc::new(MockSecretStore::unavailable()),
        );
        let manifest = Manifest::parse(
            &MANIFEST.replace("required = true", "required = false"),
            Path::new("e.toml"),
        )
        .unwrap();
        preferences.set(&manifest, "user", "ada").unwrap();
        let values = preferences.resolve(&manifest).unwrap();
        assert_eq!(values.keys().collect::<Vec<_>>(), ["user"]);
    }

    #[test]
    fn deleting_an_extension_removes_its_secrets_and_values() {
        let s = setup();
        s.preferences.set(&s.manifest, "token", "s3cret").unwrap();
        s.preferences.set(&s.manifest, "user", "ada").unwrap();
        s.secrets.set("other", "token", "keep").unwrap();
        s.preferences.remove_extension(&s.manifest).unwrap();
        assert_eq!(s.secrets.users(), ["other/token"]);
        assert!(SettingsStore::new(s.dir.path()).preference_values("gh").unwrap().is_empty());
    }

    #[test]
    fn values_of_another_extension_do_not_leak_into_a_query() {
        let s = setup();
        s.preferences.set(&s.manifest, "token", "t").unwrap();
        SettingsStore::new(s.dir.path()).set_preference("gh", "stale", "old").unwrap();
        SettingsStore::new(s.dir.path()).set_preference("other", "user", "bob").unwrap();
        let values = s.preferences.resolve(&s.manifest).unwrap();
        assert_eq!(values.keys().collect::<Vec<_>>(), ["token"]);
    }
}
