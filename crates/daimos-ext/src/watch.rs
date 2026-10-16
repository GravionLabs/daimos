//! Loading extensions into the registry and reloading the ones that change on disk.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use daimos_core::ipc::ExtensionList;

use crate::consent;
use crate::dir::{self, LoadError, MANIFEST_FILE};
use crate::manifest::Manifest;
use crate::preferences::Preferences;
use crate::provider::{Backend, ErrorProvider, providers_for};
use crate::registry::{ExtensionRecord, Registry};

/// What a change on disk concerns.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Change {
    /// One extension directory, by name.
    Extension(String),
    /// Something at the top (the directory itself): look at everything again.
    Everything,
}

/// Keeps the registry in step with an extension directory.
pub struct ExtensionHost {
    root: PathBuf,
    backend: Arc<dyn Backend>,
    registry: Arc<Registry>,
    preferences: Option<Arc<Preferences>>,
}

impl ExtensionHost {
    pub fn new(root: PathBuf, backend: Arc<dyn Backend>, registry: Arc<Registry>) -> Self {
        Self { root, backend, registry, preferences: None }
    }

    /// Lets the host list, set and clear the preference values of extensions.
    #[must_use]
    pub fn with_preferences(mut self, preferences: Arc<Preferences>) -> Self {
        self.preferences = Some(preferences);
        self
    }

    /// The extensions with their state, capabilities and preferences, as the UI shows them.
    pub fn list(&self) -> ExtensionList {
        self.registry.ipc_list_with(self.preferences.as_deref())
    }

    /// Confirms the capabilities of the extension called `name` as its manifest declares them now
    /// and loads it again, so it is enabled when this returns.
    pub fn confirm(&self, name: &str) -> Result<(), String> {
        let record = self
            .registry
            .list()
            .into_iter()
            .find(|record| record.name == name)
            .ok_or_else(|| format!("no extension named `{name}`"))?;
        consent::confirm(&self.root.join(&record.key)).map_err(|error| error.to_string())?;
        self.reload(&record.key);
        Ok(())
    }

    fn preference_target(&self, name: &str) -> Result<(Arc<Preferences>, Manifest), String> {
        let preferences =
            self.preferences.clone().ok_or_else(|| "preferences are not available".to_owned())?;
        let manifest = self
            .registry
            .manifest_of(name)
            .ok_or_else(|| format!("no extension named `{name}` is loaded"))?;
        Ok((preferences, manifest))
    }

    /// Stores the value of a preference; a password goes to the OS keyring.
    pub fn set_preference(&self, name: &str, key: &str, value: &str) -> Result<(), String> {
        let (preferences, manifest) = self.preference_target(name)?;
        preferences.set(&manifest, key, value).map_err(|error| error.to_string())
    }

    /// Removes the value of a preference.
    pub fn clear_preference(&self, name: &str, key: &str) -> Result<(), String> {
        let (preferences, manifest) = self.preference_target(name)?;
        preferences.clear(&manifest, key).map_err(|error| error.to_string())
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    /// Scans the whole directory: loads what is valid, registers an error for what is not, and
    /// drops what is gone.
    pub fn reload_all(&self) {
        let mut seen = BTreeSet::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.filter_map(Result::ok) {
                if let Some(key) = self.extension_key(&entry.path()) {
                    seen.insert(key);
                }
            }
        }
        for key in self.registry.extensions() {
            if !seen.contains(&key) {
                self.forget(&key);
            }
        }
        // In name order, so a duplicate name always fails the same (later) directory.
        for key in &seen {
            self.reload(key);
        }
    }

    /// Re-reads one extension directory and swaps its providers. A broken manifest disables the
    /// extension and registers one error item with the message; fixing the file restores it.
    pub fn reload(&self, key: &str) {
        let path = self.root.join(key);
        if self.extension_key(&path).is_none() {
            self.forget(key);
            return;
        }
        // The backend drops what it knew unless the same extension loads again: then `load`
        // replaces it in one step, so no query finds the extension missing meanwhile. Only a load
        // that goes ahead (no clash, nothing to confirm) skips the unload.
        let loaded = dir::load_extension(&path);
        let clash = loaded.as_ref().ok().and_then(|extension| {
            let name = &extension.manifest.extension.name;
            self.registry.list().into_iter().find(|other| {
                other.key != key
                    && &other.name == name
                    && other.state == crate::registry::ExtensionState::Enabled
            })
        });
        let old_name = self
            .registry
            .record(key)
            .filter(|record| record.state == crate::registry::ExtensionState::Enabled)
            .map(|record| record.name);
        let replaced_in_place = clash.is_none()
            && loaded.as_ref().is_ok_and(|extension| {
                extension.pending().is_none()
                    && old_name.as_deref() == Some(extension.manifest.extension.name.as_str())
            });
        if !replaced_in_place {
            self.unload(key);
        }
        match loaded {
            Ok(extension) => {
                let manifest = &extension.manifest;
                if let Some(other) = clash {
                    let message = LoadError::Other {
                        file: path.join(MANIFEST_FILE),
                        message: format!(
                            "the name `{}` is already used by the extension in `{}`",
                            manifest.extension.name, other.key
                        ),
                    }
                    .to_string();
                    self.fail(key, message);
                    return;
                }
                if extension.pending().is_some() {
                    log::info!("extension {key} waits for the confirmation of its capabilities");
                    let record = ExtensionRecord::needs_confirmation(key, manifest)
                        .with_consent(extension.consent.clone())
                        .with_added(extension.pending());
                    self.registry.replace(record, Vec::new());
                    return;
                }
                self.backend.load(&extension);
                let providers = providers_for(manifest, &self.backend);
                let record =
                    ExtensionRecord::enabled(key, manifest).with_consent(extension.consent.clone());
                self.registry.replace(record, providers);
                log::info!("loaded extension {key}");
            }
            Err(error) => self.fail(key, error.to_string()),
        }
    }

    /// Tells the backend that the extension in directory `key` is no longer loaded.
    fn unload(&self, key: &str) {
        if let Some(record) = self.registry.record(key) {
            self.backend.unload(&record.name);
        }
    }

    /// Drops an extension from the registry and the backend.
    fn forget(&self, key: &str) {
        self.unload(key);
        self.registry.remove(key);
    }

    fn fail(&self, key: &str, message: String) {
        log::warn!("extension {key} is disabled: {message}");
        let provider = Arc::new(ErrorProvider::new(key, &message));
        self.registry.replace(ExtensionRecord::failed(key, message), vec![provider]);
    }

    /// The directory name when `path` is a visible directory with an `extension.toml`.
    fn extension_key(&self, path: &Path) -> Option<String> {
        let name = path.file_name()?.to_string_lossy().into_owned();
        (!name.starts_with('.') && path.is_dir() && path.join(MANIFEST_FILE).exists())
            .then_some(name)
    }

    /// Loads everything now, then reloads whatever changes, `debounce` after the last change.
    /// Creates the directory when it does not exist yet. Dropping the result stops watching.
    pub fn watch(self: Arc<Self>, debounce: Duration) -> io::Result<ExtensionWatcher> {
        std::fs::create_dir_all(&self.root)?;
        let canonical = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
        let (tx, rx) = mpsc::channel::<Vec<Change>>();
        let roots = [self.root.clone(), canonical];
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if event.kind.is_access() {
                    return;
                }
                let changes = event.paths.iter().map(|path| change_of(&roots, path)).collect();
                let _ = tx.send(changes);
            })
            .map_err(io::Error::other)?;
        watcher.watch(&self.root, RecursiveMode::Recursive).map_err(io::Error::other)?;
        self.reload_all();

        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let host = Arc::clone(&self);
        let worker =
            std::thread::Builder::new().name("daimos-ext-watch".into()).spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    let mut pending = BTreeSet::new();
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(changes) => pending.extend(changes),
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    // Collect until the directory was quiet for `debounce`.
                    let mut quiet_since = Instant::now();
                    while quiet_since.elapsed() < debounce && !flag.load(Ordering::SeqCst) {
                        if let Ok(changes) = rx.recv_timeout(Duration::from_millis(20)) {
                            pending.extend(changes);
                            quiet_since = Instant::now();
                        }
                    }
                    if flag.load(Ordering::SeqCst) {
                        break;
                    }
                    if pending.contains(&Change::Everything) {
                        host.reload_all();
                    } else {
                        for change in pending {
                            if let Change::Extension(key) = change {
                                host.reload(&key);
                            }
                        }
                    }
                }
            })?;
        Ok(ExtensionWatcher {
            watcher: Mutex::new(Some(watcher)),
            stop,
            worker: Mutex::new(Some(worker)),
        })
    }
}

fn change_of(roots: &[PathBuf], path: &Path) -> Change {
    for root in roots {
        if let Ok(relative) = path.strip_prefix(root) {
            return match relative.components().next() {
                Some(first) => Change::Extension(first.as_os_str().to_string_lossy().into_owned()),
                None => Change::Everything,
            };
        }
    }
    Change::Everything
}

/// Keeps the watch alive. Dropping it (or [`stop`](Self::stop)) ends the watching.
pub struct ExtensionWatcher {
    watcher: Mutex<Option<RecommendedWatcher>>,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ExtensionWatcher {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.watcher.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = worker.join();
        }
    }
}

impl Drop for ExtensionWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use daimos_core::provider::QueryContext;
    use futures::StreamExt;
    use tokio_util::sync::CancellationToken;

    use crate::provider::StubBackend;
    use crate::registry::ExtensionState;

    use super::*;

    fn manifest(name: &str, keyword: &str) -> String {
        format!(
            "[extension]\nname = \"{name}\"\ntitle = \"T\"\nversion = \"1.0.0\"\nruntime = \"script\"\n\n\
             [[commands]]\nname = \"go\"\ntitle = \"Go\"\nkeywords = [\"{keyword}\"]\n\n\
             [runtime.script]\ncommand = [\"run.sh\"]\n"
        )
    }

    /// Writes a manifest and confirms the (empty) capabilities, so the extension may run.
    fn write(root: &Path, key: &str, content: &str) {
        write_unconfirmed(root, key, content);
        std::fs::write(
            root.join(key).join(crate::consent::CONSENT_FILE),
            r#"{"version":"1.0.0","capabilities":{}}"#,
        )
        .unwrap();
    }

    fn write_unconfirmed(root: &Path, key: &str, content: &str) {
        std::fs::create_dir_all(root.join(key)).unwrap();
        std::fs::write(root.join(key).join(MANIFEST_FILE), content).unwrap();
    }

    fn with_capabilities(manifest: &str, capabilities: &str) -> String {
        format!("{manifest}\n[capabilities]\n{capabilities}\n")
    }

    fn host(root: &Path) -> Arc<ExtensionHost> {
        Arc::new(ExtensionHost::new(
            root.to_path_buf(),
            Arc::new(StubBackend),
            Arc::new(Registry::new()),
        ))
    }

    fn keywords_of(registry: &Registry, key: &str) -> Vec<String> {
        registry
            .providers_of(key)
            .iter()
            .flat_map(|p| p.keywords().into_iter().map(str::to_owned).collect::<Vec<_>>())
            .collect()
    }

    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if check() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("timed out waiting for: {what}");
    }

    fn items(registry: &Registry, key: &str, text: &str) -> usize {
        let ctx = QueryContext {
            text: text.into(),
            keyword_routed: false,
            cancel: CancellationToken::new(),
        };
        registry
            .providers_of(key)
            .iter()
            .map(|p| futures::executor::block_on(p.query(ctx.clone()).collect::<Vec<_>>()))
            .map(|batches| batches.iter().map(|b| b.items.len()).sum::<usize>())
            .sum()
    }

    #[test]
    fn reload_all_loads_valid_extensions_and_flags_broken_ones() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "good", &manifest("good", "g"));
        write(root.path(), "bad", "[extension]\nname = \n");
        let host = host(root.path());
        host.reload_all();
        let list = host.registry().list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].key, "bad");
        assert!(
            matches!(&list[0].state, ExtensionState::Error(m) if m.contains("extension.toml:2"))
        );
        assert_eq!(list[1].state, ExtensionState::Enabled);
        assert_eq!(keywords_of(host.registry(), "good"), ["g"]);
    }

    #[test]
    fn editing_a_keyword_swaps_the_provider() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "ext", &manifest("ext", "old"));
        let host = host(root.path());
        host.reload_all();
        assert_eq!(keywords_of(host.registry(), "ext"), ["old"]);
        write(root.path(), "ext", &manifest("ext", "new"));
        host.reload("ext");
        assert_eq!(keywords_of(host.registry(), "ext"), ["new"]);
    }

    #[test]
    fn a_broken_manifest_yields_one_error_item_and_fixing_it_restores_the_extension() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "ext", &manifest("ext", "k"));
        let host = host(root.path());
        host.reload_all();
        let registry = host.registry();
        assert_eq!(items(registry, "ext", ""), 0);

        write(
            root.path(),
            "ext",
            &manifest("ext", "k").replace("name = \"ext\"", "name = \"E X\""),
        );
        host.reload("ext");
        let record = registry.record("ext").unwrap();
        let ExtensionState::Error(message) = &record.state else { panic!("{record:?}") };
        assert!(message.contains("extension.name"), "{message}");
        assert_eq!(registry.providers_of("ext").len(), 1);
        assert_eq!(items(registry, "ext", ""), 1);

        write(root.path(), "ext", &manifest("ext", "k"));
        host.reload("ext");
        assert_eq!(registry.record("ext").unwrap().state, ExtensionState::Enabled);
        assert_eq!(keywords_of(registry, "ext"), ["k"]);
        assert_eq!(items(registry, "ext", ""), 0);
    }

    #[test]
    fn an_extension_without_consent_waits_and_runs_nothing() {
        let root = tempfile::tempdir().unwrap();
        write_unconfirmed(root.path(), "ext", &manifest("ext", "k"));
        let host = host(root.path());
        host.reload_all();
        let registry = host.registry();
        let record = registry.record("ext").unwrap();
        assert_eq!(record.state, ExtensionState::NeedsConfirmation);
        assert_eq!(record.state.label(), "needs-confirmation");
        assert!(registry.providers_of("ext").is_empty());

        crate::consent::confirm(&root.path().join("ext")).unwrap();
        host.reload("ext");
        assert_eq!(registry.record("ext").unwrap().state, ExtensionState::Enabled);
        assert_eq!(keywords_of(registry, "ext"), ["k"]);
    }

    #[test]
    fn an_update_that_adds_a_capability_is_disabled_until_confirmed_again() {
        let root = tempfile::tempdir().unwrap();
        let base = manifest("ext", "k");
        let first = with_capabilities(
            &base,
            "network = [\"a.com\"]\nfilesystem = [\"/x:ro\"]\nkeyring = [\"t\"]",
        );
        write_unconfirmed(root.path(), "ext", &first);
        crate::consent::confirm(&root.path().join("ext")).unwrap();
        let host = host(root.path());
        host.reload_all();
        let registry = host.registry();
        assert_eq!(registry.record("ext").unwrap().state, ExtensionState::Enabled);

        // Each addition alone puts the extension back to pending; confirming enables it again.
        for added in [
            "network = [\"a.com\", \"b.com\"]\nfilesystem = [\"/x:ro\"]\nkeyring = [\"t\"]",
            "network = [\"a.com\"]\nfilesystem = [\"/x:ro\", \"/y:ro\"]\nkeyring = [\"t\"]",
            "network = [\"a.com\"]\nfilesystem = [\"/x:rw\"]\nkeyring = [\"t\"]",
            "network = [\"a.com\"]\nfilesystem = [\"/x:ro\"]\nkeyring = [\"t\", \"u\"]",
            "network = [\"a.com\"]\nfilesystem = [\"/x:ro\"]\nkeyring = [\"t\"]\nclipboard = \"read\"",
        ] {
            write_unconfirmed(root.path(), "ext", &with_capabilities(&base, added));
            host.reload("ext");
            assert_eq!(
                registry.record("ext").unwrap().state,
                ExtensionState::NeedsConfirmation,
                "{added}"
            );
            assert!(registry.providers_of("ext").is_empty());
            crate::consent::confirm(&root.path().join("ext")).unwrap();
            host.reload("ext");
            assert_eq!(registry.record("ext").unwrap().state, ExtensionState::Enabled, "{added}");
            // Back to the first set, which only removes: still enabled.
            write_unconfirmed(root.path(), "ext", &first);
            host.reload("ext");
            assert_eq!(registry.record("ext").unwrap().state, ExtensionState::Enabled);
            crate::consent::confirm(&root.path().join("ext")).unwrap();
        }
    }

    #[test]
    fn an_update_that_only_removes_a_capability_stays_enabled() {
        let root = tempfile::tempdir().unwrap();
        let base = manifest("ext", "k");
        write_unconfirmed(
            root.path(),
            "ext",
            &with_capabilities(
                &base,
                "network = [\"a.com\", \"b.com\"]\nclipboard = \"read-write\"",
            ),
        );
        crate::consent::confirm(&root.path().join("ext")).unwrap();
        let host = host(root.path());
        host.reload_all();
        write_unconfirmed(
            root.path(),
            "ext",
            &with_capabilities(&base, "network = [\"a.com\"]\nclipboard = \"read\""),
        );
        host.reload("ext");
        assert_eq!(host.registry().record("ext").unwrap().state, ExtensionState::Enabled);
    }

    fn host_with_preferences(
        root: &Path,
    ) -> (Arc<ExtensionHost>, Arc<crate::secrets::MockSecretStore>) {
        let secrets = Arc::new(crate::secrets::MockSecretStore::new());
        let preferences = Arc::new(Preferences::new(
            daimos_core::settings::SettingsStore::new(root.join(".config")),
            secrets.clone(),
        ));
        let host = ExtensionHost::new(
            root.to_path_buf(),
            Arc::new(StubBackend),
            Arc::new(Registry::new()),
        )
        .with_preferences(preferences);
        (Arc::new(host), secrets)
    }

    #[test]
    fn confirming_by_name_enables_the_extension_and_the_list_shows_it() {
        let root = tempfile::tempdir().unwrap();
        write_unconfirmed(root.path(), "folder", &manifest("ext", "k"));
        let host = host(root.path());
        host.reload_all();
        let before = host.list().extensions.remove(0);
        assert_eq!(before.state, "needs-confirmation");
        assert!(before.confirmed.is_none());
        assert_eq!(before.added.as_ref().map(|a| a.network.len()), Some(0));
        assert!(host.registry().providers_of("folder").is_empty());

        host.confirm("ext").unwrap();
        let after = host.list().extensions.remove(0);
        assert_eq!(after.state, "enabled");
        assert!(after.confirmed.is_some() && after.added.is_none());
        assert_eq!(keywords_of(host.registry(), "folder"), ["k"]);

        let error = host.confirm("nope").unwrap_err();
        assert!(error.contains("no extension named `nope`"), "{error}");
    }

    #[test]
    fn confirming_an_update_lists_the_added_capability_first() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "ext", &manifest("ext", "k"));
        let host = host(root.path());
        host.reload_all();
        write_unconfirmed(
            root.path(),
            "ext",
            &with_capabilities(&manifest("ext", "k"), "network = [\"new.example\"]"),
        );
        host.reload("ext");
        let info = host.list().extensions.remove(0);
        assert_eq!(info.state, "needs-confirmation");
        assert_eq!(info.added.unwrap().network, ["new.example"]);
        assert!(info.confirmed.unwrap().network.is_empty());
        host.confirm("ext").unwrap();
        assert_eq!(host.list().extensions[0].state, "enabled");
    }

    #[test]
    fn preferences_are_set_and_cleared_by_extension_and_key() {
        let root = tempfile::tempdir().unwrap();
        let source = format!(
            "{}\n[[preferences]]\nname = \"token\"\ntitle = \"T\"\ntype = \"password\"\n",
            manifest("ext", "k")
        );
        write(root.path(), "ext", &source);
        let (host, secrets) = host_with_preferences(root.path());
        host.reload_all();
        host.set_preference("ext", "token", "pw").unwrap();
        assert_eq!(secrets.users(), ["ext/token"]);
        assert!(host.list().extensions[0].preferences[0].is_set);
        host.clear_preference("ext", "token").unwrap();
        assert!(secrets.users().is_empty());
        assert!(!host.list().extensions[0].preferences[0].is_set);

        assert!(host.set_preference("ext", "nope", "x").unwrap_err().contains("no preference"));
        assert!(host.set_preference("other", "token", "x").unwrap_err().contains("no extension"));
        // A host without preferences refuses.
        let plain = self::host(root.path());
        plain.reload_all();
        assert!(plain.set_preference("ext", "token", "x").is_err());
    }

    #[test]
    fn a_deleted_extension_is_removed() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "ext", &manifest("ext", "k"));
        let host = host(root.path());
        host.reload_all();
        std::fs::remove_dir_all(root.path().join("ext")).unwrap();
        host.reload("ext");
        assert!(host.registry().list().is_empty());
    }

    #[test]
    fn a_name_used_twice_disables_the_later_directory() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "a", &manifest("same", "x"));
        write(root.path(), "b", &manifest("same", "y"));
        let host = host(root.path());
        host.reload_all();
        let list = host.registry().list();
        assert_eq!(list[0].state, ExtensionState::Enabled);
        assert!(matches!(&list[1].state, ExtensionState::Error(m) if m.contains("already used")));
    }

    #[test]
    fn reload_all_drops_extensions_that_vanished() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "gone", &manifest("gone", "g"));
        let host = host(root.path());
        host.reload_all();
        std::fs::remove_dir_all(root.path().join("gone")).unwrap();
        host.reload_all();
        assert!(host.registry().list().is_empty());
    }

    #[test]
    fn changes_map_to_extension_directories() {
        let roots = [PathBuf::from("/r"), PathBuf::from("/real/r")];
        assert_eq!(
            change_of(&roots, Path::new("/r/ext/extension.toml")),
            Change::Extension("ext".into())
        );
        assert_eq!(change_of(&roots, Path::new("/real/r/ext")), Change::Extension("ext".into()));
        assert_eq!(change_of(&roots, Path::new("/r")), Change::Everything);
        assert_eq!(change_of(&roots, Path::new("/elsewhere/x")), Change::Everything);
    }

    #[test]
    fn the_watcher_reloads_a_changed_extension_without_a_restart() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "ext", &manifest("ext", "old"));
        let host = host(root.path());
        let watcher = Arc::clone(&host).watch(Duration::from_millis(50)).unwrap();
        let registry = Arc::clone(host.registry());
        assert_eq!(keywords_of(&registry, "ext"), ["old"]);

        write(root.path(), "ext", &manifest("ext", "new"));
        eventually("the new keyword", || keywords_of(&registry, "ext") == ["new"]);

        write(root.path(), "ext", "not [valid");
        eventually("the error state", || {
            matches!(registry.record("ext").map(|r| r.state), Some(ExtensionState::Error(_)))
        });
        assert_eq!(registry.providers_of("ext").len(), 1);

        write(root.path(), "ext", &manifest("ext", "fixed"));
        eventually("the fixed extension", || keywords_of(&registry, "ext") == ["fixed"]);

        write(root.path(), "added", &manifest("added", "a"));
        eventually("a new extension", || registry.record("added").is_some());

        // A new version that asks for more is held back, and confirming releases it, both seen by
        // the watcher alone.
        let wider = with_capabilities(&manifest("ext", "wide"), "network = [\"b.com\"]");
        write_unconfirmed(root.path(), "ext", &wider);
        eventually("the pending state", || {
            registry.record("ext").map(|r| r.state) == Some(ExtensionState::NeedsConfirmation)
        });
        crate::consent::confirm(&root.path().join("ext")).unwrap();
        eventually("the confirmed extension", || keywords_of(&registry, "ext") == ["wide"]);
        watcher.stop();
    }

    #[test]
    fn watching_creates_a_missing_directory() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("daimos").join("extensions");
        let host = host(&root);
        let _watcher = Arc::clone(&host).watch(Duration::from_millis(50)).unwrap();
        assert!(root.is_dir());
        assert!(host.registry().list().is_empty());
    }
}
