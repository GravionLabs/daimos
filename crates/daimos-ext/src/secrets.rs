//! Secrets of extensions (`password` preferences) in the OS keyring.
//!
//! An entry has the service `daimos` and the user `<extension name>/<preference name>`, so a secret
//! can be found and removed by hand in Seahorse, Credential Manager or Keychain Access.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

/// The keyring service of every entry.
pub const SERVICE: &str = "daimos";

/// The keyring user of a secret.
pub fn secret_user(extension: &str, name: &str) -> String {
    format!("{extension}/{name}")
}

/// Why a secret could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// There is no keyring to talk to, for example no running Secret Service on Linux.
    Unavailable(String),
    /// The keyring refused or failed.
    Failed(String),
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(detail) => write!(
                f,
                "the OS keyring is not available ({detail}); on Linux start a Secret Service such \
                 as GNOME Keyring or KWallet"
            ),
            Self::Failed(detail) => write!(f, "the OS keyring failed: {detail}"),
        }
    }
}

impl std::error::Error for SecretError {}

/// Where password preferences are kept.
pub trait SecretStore: Send + Sync {
    /// The secret, `None` when none is stored.
    fn get(&self, extension: &str, name: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, extension: &str, name: &str, value: &str) -> Result<(), SecretError>;
    /// Removes the secret; one that is not stored is fine.
    fn delete(&self, extension: &str, name: &str) -> Result<(), SecretError>;
}

/// The keyring of the operating system: Secret Service, Credential Manager or Keychain.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsSecretStore;

fn convert(error: keyring::Error) -> SecretError {
    match error {
        keyring::Error::NoDefaultStore
        | keyring::Error::NoStorageAccess(_)
        | keyring::Error::PlatformFailure(_) => SecretError::Unavailable(error.to_string()),
        other => SecretError::Failed(other.to_string()),
    }
}

fn entry(extension: &str, name: &str) -> Result<keyring::Entry, SecretError> {
    keyring::Entry::new(SERVICE, &secret_user(extension, name)).map_err(convert)
}

impl SecretStore for OsSecretStore {
    fn get(&self, extension: &str, name: &str) -> Result<Option<String>, SecretError> {
        match entry(extension, name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(convert(error)),
        }
    }

    fn set(&self, extension: &str, name: &str, value: &str) -> Result<(), SecretError> {
        entry(extension, name)?.set_password(value).map_err(convert)
    }

    fn delete(&self, extension: &str, name: &str) -> Result<(), SecretError> {
        match entry(extension, name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(convert(error)),
        }
    }
}

/// An in-memory keyring for tests; it can also play a machine without a keyring.
#[derive(Debug, Default)]
pub struct MockSecretStore {
    secrets: Mutex<BTreeMap<String, String>>,
    unavailable: bool,
}

impl MockSecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// A store whose every call fails as a missing keyring service does.
    pub fn unavailable() -> Self {
        Self { unavailable: true, ..Self::default() }
    }

    /// The keyring users that hold a secret, sorted.
    pub fn users(&self) -> Vec<String> {
        self.secrets.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    fn check(&self) -> Result<(), SecretError> {
        if self.unavailable {
            Err(SecretError::Unavailable("no Secret Service on the session bus".into()))
        } else {
            Ok(())
        }
    }
}

impl SecretStore for MockSecretStore {
    fn get(&self, extension: &str, name: &str) -> Result<Option<String>, SecretError> {
        self.check()?;
        let secrets = self.secrets.lock().unwrap_or_else(|e| e.into_inner());
        Ok(secrets.get(&secret_user(extension, name)).cloned())
    }

    fn set(&self, extension: &str, name: &str, value: &str) -> Result<(), SecretError> {
        self.check()?;
        let mut secrets = self.secrets.lock().unwrap_or_else(|e| e.into_inner());
        secrets.insert(secret_user(extension, name), value.to_owned());
        Ok(())
    }

    fn delete(&self, extension: &str, name: &str) -> Result<(), SecretError> {
        self.check()?;
        let mut secrets = self.secrets.lock().unwrap_or_else(|e| e.into_inner());
        secrets.remove(&secret_user(extension, name));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mock_stores_under_the_extension_and_preference_name() {
        let store = MockSecretStore::new();
        assert_eq!(store.get("gh", "token").unwrap(), None);
        store.set("gh", "token", "s3cret").unwrap();
        store.set("other", "token", "x").unwrap();
        assert_eq!(store.get("gh", "token").unwrap().as_deref(), Some("s3cret"));
        assert_eq!(store.users(), ["gh/token", "other/token"]);
        store.set("gh", "token", "new").unwrap();
        assert_eq!(store.get("gh", "token").unwrap().as_deref(), Some("new"));
        store.delete("gh", "token").unwrap();
        store.delete("gh", "token").unwrap();
        assert_eq!(store.users(), ["other/token"]);
    }

    #[test]
    fn a_missing_keyring_service_has_a_clear_message() {
        let store = MockSecretStore::unavailable();
        for result in
            [store.get("a", "b").map(drop), store.set("a", "b", "c"), store.delete("a", "b")]
        {
            let message = result.unwrap_err().to_string();
            assert!(message.contains("OS keyring is not available"), "{message}");
            assert!(message.contains("Secret Service"), "{message}");
        }
    }

    #[test]
    fn the_user_is_extension_slash_preference() {
        assert_eq!(secret_user("github-repos", "token"), "github-repos/token");
        assert_eq!(SERVICE, "daimos");
    }

    #[test]
    fn keyring_errors_are_sorted_into_unavailable_and_failed() {
        assert!(matches!(convert(keyring::Error::NoDefaultStore), SecretError::Unavailable(_)));
        assert!(matches!(
            convert(keyring::Error::TooLong("user".into(), 1)),
            SecretError::Failed(_)
        ));
    }
}
