//! The providers of the loaded extensions, swappable while the app runs.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use daimos_core::ipc::{ExtensionInfo, ExtensionList};
use daimos_core::provider::Provider;

use crate::manifest::{Manifest, Runtime};

/// Whether an extension runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionState {
    Enabled,
    /// Disabled because it failed to load; carries the message shown to the user.
    Error(String),
}

impl ExtensionState {
    /// `enabled` or `error`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Error(_) => "error",
        }
    }
}

/// What `daimos ext list` shows about one extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionRecord {
    /// The extension's directory name; the registry key.
    pub key: String,
    /// The manifest name, or the directory name when the manifest is broken.
    pub name: String,
    pub version: Option<String>,
    pub runtime: Option<Runtime>,
    pub state: ExtensionState,
}

impl ExtensionRecord {
    pub fn enabled(key: &str, manifest: &Manifest) -> Self {
        Self {
            key: key.to_owned(),
            name: manifest.extension.name.clone(),
            version: Some(manifest.extension.version.clone()),
            runtime: Some(manifest.extension.runtime),
            state: ExtensionState::Enabled,
        }
    }

    pub fn failed(key: &str, message: String) -> Self {
        Self {
            key: key.to_owned(),
            name: key.to_owned(),
            version: None,
            runtime: None,
            state: ExtensionState::Error(message),
        }
    }
}

struct Entry {
    record: ExtensionRecord,
    providers: Vec<Arc<dyn Provider>>,
}

/// Holds the providers and the state of each extension, keyed by directory name.
#[derive(Default)]
pub struct Registry {
    extensions: RwLock<BTreeMap<String, Entry>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the record and providers of an extension, replacing what it had.
    pub fn replace(&self, record: ExtensionRecord, providers: Vec<Arc<dyn Provider>>) {
        self.extensions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(record.key.clone(), Entry { record, providers });
    }

    /// Drops an extension. Returns whether it had an entry.
    pub fn remove(&self, key: &str) -> bool {
        self.extensions.write().unwrap_or_else(|e| e.into_inner()).remove(key).is_some()
    }

    /// Every provider, extensions in key order and commands in manifest order.
    pub fn providers(&self) -> Vec<Arc<dyn Provider>> {
        let extensions = self.extensions.read().unwrap_or_else(|e| e.into_inner());
        extensions.values().flat_map(|entry| entry.providers.iter()).cloned().collect()
    }

    /// The providers of one extension.
    pub fn providers_of(&self, key: &str) -> Vec<Arc<dyn Provider>> {
        let extensions = self.extensions.read().unwrap_or_else(|e| e.into_inner());
        extensions.get(key).map(|entry| entry.providers.clone()).unwrap_or_default()
    }

    /// Keys of the extensions that have an entry.
    pub fn extensions(&self) -> Vec<String> {
        self.extensions.read().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    /// The records of all extensions in key order.
    pub fn list(&self) -> Vec<ExtensionRecord> {
        let extensions = self.extensions.read().unwrap_or_else(|e| e.into_inner());
        extensions.values().map(|entry| entry.record.clone()).collect()
    }

    /// The extensions as `daimos ext list` answers over the socket.
    pub fn ipc_list(&self) -> ExtensionList {
        let extensions = self
            .list()
            .into_iter()
            .map(|record| ExtensionInfo {
                name: record.name,
                version: record.version,
                runtime: record.runtime.map(|r| r.as_str().to_owned()),
                state: record.state.label().to_owned(),
                message: match record.state {
                    ExtensionState::Error(message) => Some(message),
                    ExtensionState::Enabled => None,
                },
            })
            .collect();
        ExtensionList { extensions }
    }

    pub fn record(&self, key: &str) -> Option<ExtensionRecord> {
        let extensions = self.extensions.read().unwrap_or_else(|e| e.into_inner());
        extensions.get(key).map(|entry| entry.record.clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::provider::tests::manifest;
    use crate::provider::{Backend, StubBackend, providers_for};

    use super::*;

    /// Puts an extension called `name` into the registry.
    fn put(registry: &Registry, name: &str, keywords: &str) {
        let manifest = manifest(name, keywords);
        let providers = providers_for(&manifest, &(Arc::new(StubBackend) as Arc<dyn Backend>));
        registry.replace(ExtensionRecord::enabled(name, &manifest), providers);
    }

    fn ids(registry: &Registry) -> Vec<String> {
        registry.providers().iter().map(|p| p.id().to_owned()).collect()
    }

    #[test]
    fn providers_of_all_extensions_are_listed_in_name_order() {
        let registry = Registry::new();
        put(&registry, "zeta", r#"["z"]"#);
        put(&registry, "alpha", r#"["a"]"#);
        assert_eq!(ids(&registry), ["alpha.search", "alpha.open", "zeta.search", "zeta.open"]);
        assert_eq!(registry.extensions(), ["alpha", "zeta"]);
    }

    #[test]
    fn replace_swaps_the_providers_of_one_extension() {
        let registry = Registry::new();
        put(&registry, "gh", r#"["gh"]"#);
        put(&registry, "other", r#"["o"]"#);
        put(&registry, "gh", r#"["git"]"#);
        let gh = registry.providers_of("gh");
        assert_eq!(gh.len(), 2);
        assert_eq!(gh[0].keywords(), ["git"]);
        assert_eq!(registry.providers().len(), 4);
    }

    #[test]
    fn remove_drops_only_that_extension() {
        let registry = Registry::new();
        put(&registry, "gh", r#"["gh"]"#);
        put(&registry, "other", r#"["o"]"#);
        assert!(registry.remove("gh"));
        assert_eq!(ids(&registry), ["other.search", "other.open"]);
        assert!(registry.providers_of("gh").is_empty());
    }

    #[test]
    fn removing_an_unknown_extension_changes_nothing() {
        let registry = Registry::new();
        assert!(!registry.remove("nope"));
        assert!(registry.providers().is_empty());
    }

    #[test]
    fn the_list_has_name_version_runtime_and_state() {
        let registry = Registry::new();
        put(&registry, "gh", r#"["gh"]"#);
        registry.replace(ExtensionRecord::failed("broken", "bad".into()), Vec::new());
        let list = registry.list();
        assert_eq!(list.len(), 2);
        let broken = &list[0];
        assert_eq!(
            (broken.name.as_str(), broken.version.clone(), broken.runtime),
            ("broken", None, None)
        );
        assert_eq!(broken.state, ExtensionState::Error("bad".into()));
        let gh = registry.record("gh").unwrap();
        assert_eq!(gh.version.as_deref(), Some("1.0.0"));
        assert_eq!(gh.runtime, Some(Runtime::Script));
        assert_eq!(gh.state.label(), "enabled");
        assert_eq!(broken.state.label(), "error");
    }

    #[test]
    fn the_ipc_list_carries_state_and_the_error_message() {
        let registry = Registry::new();
        assert!(registry.ipc_list().extensions.is_empty());
        put(&registry, "gh", r#"["gh"]"#);
        registry.replace(ExtensionRecord::failed("broken", "bad".into()), Vec::new());
        let list = registry.ipc_list().extensions;
        assert_eq!(list[0].name, "broken");
        assert_eq!((list[0].state.as_str(), list[0].message.as_deref()), ("error", Some("bad")));
        assert_eq!(list[0].runtime, None);
        assert_eq!(list[1].name, "gh");
        assert_eq!(list[1].runtime.as_deref(), Some("script"));
        assert_eq!(list[1].version.as_deref(), Some("1.0.0"));
        assert_eq!((list[1].state.as_str(), list[1].message.as_deref()), ("enabled", None));
    }
}
