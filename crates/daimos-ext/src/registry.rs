//! The providers of the loaded extensions, swappable while the app runs.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use daimos_core::ipc::{
    CapabilityList, ExtensionInfo, ExtensionList, FilesystemAccess, PreferenceInfo,
};
use daimos_core::provider::Provider;

use crate::consent::ConsentRecord;
use crate::manifest::{
    Capabilities, ClipboardAccess, Manifest, PreferenceKind, Runtime, split_filesystem,
};
use crate::preferences::Preferences;

/// Whether an extension runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionState {
    Enabled,
    /// Not running until the user confirms its capabilities, the first time or after an update
    /// that added one.
    NeedsConfirmation,
    /// Disabled because it failed to load; carries the message shown to the user.
    Error(String),
}

impl ExtensionState {
    /// `enabled` or `error`.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::NeedsConfirmation => "needs-confirmation",
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
    /// The manifest, when it loaded.
    pub manifest: Option<Manifest>,
    /// What the user confirmed.
    pub consent: Option<ConsentRecord>,
    /// What waits for a confirmation, in the `needs-confirmation` state.
    pub added: Option<Capabilities>,
}

/// The wire form of capabilities.
pub fn capability_list(capabilities: &Capabilities) -> CapabilityList {
    CapabilityList {
        network: capabilities.network.clone(),
        filesystem: capabilities
            .filesystem
            .iter()
            .map(|entry| {
                let (path, write) = split_filesystem(entry);
                FilesystemAccess { path: path.to_owned(), write }
            })
            .collect(),
        clipboard: capabilities.clipboard.map(|access| {
            match access {
                ClipboardAccess::Read => "read",
                ClipboardAccess::Write => "write",
                ClipboardAccess::ReadWrite => "read-write",
            }
            .to_owned()
        }),
        keyring: capabilities.keyring.clone(),
    }
}

impl ExtensionRecord {
    pub fn enabled(key: &str, manifest: &Manifest) -> Self {
        Self {
            key: key.to_owned(),
            name: manifest.extension.name.clone(),
            version: Some(manifest.extension.version.clone()),
            runtime: Some(manifest.extension.runtime),
            state: ExtensionState::Enabled,
            manifest: Some(manifest.clone()),
            consent: None,
            added: None,
        }
    }

    /// Adds the consent the extension was loaded with.
    #[must_use]
    pub fn with_consent(mut self, consent: Option<ConsentRecord>) -> Self {
        self.consent = consent;
        self
    }

    /// Adds what waits for a confirmation.
    #[must_use]
    pub fn with_added(mut self, added: Option<Capabilities>) -> Self {
        self.added = added;
        self
    }

    /// An extension that waits for the user to confirm its capabilities.
    pub fn needs_confirmation(key: &str, manifest: &Manifest) -> Self {
        Self { state: ExtensionState::NeedsConfirmation, ..Self::enabled(key, manifest) }
    }

    pub fn failed(key: &str, message: String) -> Self {
        Self {
            key: key.to_owned(),
            name: key.to_owned(),
            version: None,
            runtime: None,
            state: ExtensionState::Error(message),
            manifest: None,
            consent: None,
            added: None,
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

    /// The extensions as `daimos ext list` answers over the socket. Preference values are not
    /// looked up; use [`ipc_list_with`](Self::ipc_list_with) for those.
    pub fn ipc_list(&self) -> ExtensionList {
        self.ipc_list_with(None)
    }

    /// Like [`ipc_list`](Self::ipc_list), with the stored preference values from `preferences`.
    /// The value of a password is never included, only whether one is set.
    pub fn ipc_list_with(&self, preferences: Option<&Preferences>) -> ExtensionList {
        let extensions =
            self.list().into_iter().map(|record| info_of(record, preferences)).collect();
        ExtensionList { extensions }
    }

    /// The manifest of the extension called `name` (its manifest name).
    pub fn manifest_of(&self, name: &str) -> Option<Manifest> {
        self.list().into_iter().find(|record| record.name == name).and_then(|r| r.manifest)
    }

    pub fn record(&self, key: &str) -> Option<ExtensionRecord> {
        let extensions = self.extensions.read().unwrap_or_else(|e| e.into_inner());
        extensions.get(key).map(|entry| entry.record.clone())
    }
}

fn info_of(record: ExtensionRecord, preferences: Option<&Preferences>) -> ExtensionInfo {
    let manifest = record.manifest.as_ref();
    let status = manifest.zip(preferences).and_then(|(m, p)| p.status(m).ok()).unwrap_or_default();
    let stored = manifest.zip(preferences).and_then(|(m, p)| p.values(m).ok()).unwrap_or_default();
    let preferences = manifest
        .map(|m| {
            m.preferences
                .iter()
                .map(|preference| PreferenceInfo {
                    name: preference.name.clone(),
                    title: preference.title.clone(),
                    kind: match preference.kind {
                        PreferenceKind::Text => "text",
                        PreferenceKind::Password => "password",
                        PreferenceKind::Number => "number",
                        PreferenceKind::Checkbox => "checkbox",
                    }
                    .to_owned(),
                    required: preference.required,
                    is_set: status.iter().any(|s| s.name == preference.name && s.is_set),
                    value: stored.get(&preference.name).cloned(),
                })
                .collect()
        })
        .unwrap_or_default();
    ExtensionInfo {
        name: record.name,
        title: manifest.map(|m| m.extension.title.clone()),
        version: record.version,
        runtime: record.runtime.map(|r| r.as_str().to_owned()),
        sandboxed: record.runtime == Some(Runtime::Wasm),
        state: record.state.label().to_owned(),
        message: match record.state {
            ExtensionState::Error(message) => Some(message),
            ExtensionState::Enabled | ExtensionState::NeedsConfirmation => None,
        },
        capabilities: manifest.map(|m| capability_list(&m.capabilities)).unwrap_or_default(),
        confirmed: record.consent.as_ref().map(|c| capability_list(&c.capabilities)),
        added: record.added.as_ref().map(capability_list),
        preferences,
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

    fn wasm_manifest() -> Manifest {
        let source = "[extension]\nname = \"w\"\ntitle = \"Wasm One\"\nversion = \"2.0.0\"\n\
            runtime = \"wasm\"\n\n[[commands]]\nname = \"s\"\ntitle = \"S\"\n\n\
            [runtime.wasm]\ncomponent = \"main.wasm\"\n\n[capabilities]\n\
            network = [\"a.com\"]\nfilesystem = [\"/x:rw\", \"/y\"]\nclipboard = \"read-write\"\n\
            keyring = [\"k\"]\n\n[[preferences]]\nname = \"token\"\ntitle = \"Token\"\n\
            type = \"password\"\nrequired = true\n\n[[preferences]]\nname = \"user\"\n\
            title = \"User\"\ntype = \"text\"\n";
        Manifest::parse(source, std::path::Path::new("e.toml")).unwrap()
    }

    #[test]
    fn the_ipc_list_labels_scripts_unsandboxed_and_wasm_sandboxed() {
        let registry = Registry::new();
        put(&registry, "gh", r#"["gh"]"#);
        registry.replace(ExtensionRecord::enabled("w", &wasm_manifest()), Vec::new());
        registry.replace(ExtensionRecord::failed("broken", "bad".into()), Vec::new());
        let list = registry.ipc_list().extensions;
        let by_name = |name: &str| list.iter().find(|e| e.name == name).unwrap();
        assert!(!by_name("gh").sandboxed);
        assert!(by_name("w").sandboxed);
        assert!(!by_name("broken").sandboxed, "no runtime, no label");
        assert_eq!(by_name("w").title.as_deref(), Some("Wasm One"));
    }

    #[test]
    fn the_ipc_list_carries_declared_and_confirmed_capabilities() {
        let registry = Registry::new();
        let manifest = wasm_manifest();
        let confirmed = crate::consent::ConsentRecord {
            version: "1.0.0".into(),
            capabilities: Capabilities { network: vec!["a.com".into()], ..Capabilities::default() },
        };
        let added = manifest.capabilities.diff(&confirmed.capabilities);
        let record = ExtensionRecord::needs_confirmation("w", &manifest)
            .with_consent(Some(confirmed))
            .with_added(Some(added));
        registry.replace(record, Vec::new());
        let info = registry.ipc_list().extensions.remove(0);
        assert_eq!(info.state, "needs-confirmation");
        assert_eq!(info.capabilities.network, ["a.com"]);
        assert_eq!(
            info.capabilities
                .filesystem
                .iter()
                .map(|f| (f.path.as_str(), f.write))
                .collect::<Vec<_>>(),
            [("/x", true), ("/y", false)]
        );
        assert_eq!(info.capabilities.clipboard.as_deref(), Some("read-write"));
        assert_eq!(info.capabilities.keyring, ["k"]);
        assert_eq!(info.confirmed.unwrap().network, ["a.com"]);
        let added = info.added.unwrap();
        assert!(added.network.is_empty());
        assert_eq!(added.keyring, ["k"]);
        assert_eq!(added.filesystem.len(), 2);
    }

    #[test]
    fn preferences_are_listed_and_a_password_value_is_never_serialized() {
        use crate::secrets::{MockSecretStore, SecretStore};
        use daimos_core::settings::SettingsStore;
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MockSecretStore::new());
        let preferences = Preferences::new(SettingsStore::new(dir.path()), secrets.clone());
        let manifest = wasm_manifest();
        let registry = Registry::new();
        registry.replace(ExtensionRecord::enabled("w", &manifest), Vec::new());

        let before = registry.ipc_list_with(Some(&preferences)).extensions.remove(0);
        assert_eq!(before.preferences.len(), 2);
        assert!(before.preferences.iter().all(|p| !p.is_set));
        assert_eq!(
            (before.preferences[0].kind.as_str(), before.preferences[0].required),
            ("password", true)
        );

        preferences.set(&manifest, "token", "hunter2-secret").unwrap();
        preferences.set(&manifest, "user", "ada").unwrap();
        let list = registry.ipc_list_with(Some(&preferences));
        let after = &list.extensions[0];
        assert!(after.preferences[0].is_set);
        assert_eq!(after.preferences[0].value, None);
        assert_eq!(after.preferences[1].value.as_deref(), Some("ada"));
        let json = serde_json::to_string(&list).unwrap();
        assert!(!json.contains("hunter2-secret"), "{json}");
        assert!(!format!("{list:?}").contains("hunter2-secret"));
        assert_eq!(secrets.get("w", "token").unwrap().as_deref(), Some("hunter2-secret"));
    }
}
