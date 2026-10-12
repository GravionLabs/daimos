//! The providers of the loaded extensions, swappable while the app runs.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use daimos_core::provider::Provider;

/// Holds the providers of each extension, keyed by extension name.
#[derive(Default)]
pub struct Registry {
    extensions: RwLock<BTreeMap<String, Vec<Arc<dyn Provider>>>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the providers of `extension`, replacing the ones it had.
    pub fn replace(&self, extension: &str, providers: Vec<Arc<dyn Provider>>) {
        self.extensions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(extension.to_owned(), providers);
    }

    /// Drops the providers of `extension`. Returns whether it had any entry.
    pub fn remove(&self, extension: &str) -> bool {
        self.extensions.write().unwrap_or_else(|e| e.into_inner()).remove(extension).is_some()
    }

    /// Every provider, extensions in name order and commands in manifest order.
    pub fn providers(&self) -> Vec<Arc<dyn Provider>> {
        let extensions = self.extensions.read().unwrap_or_else(|e| e.into_inner());
        extensions.values().flatten().cloned().collect()
    }

    /// The providers of one extension.
    pub fn providers_of(&self, extension: &str) -> Vec<Arc<dyn Provider>> {
        let extensions = self.extensions.read().unwrap_or_else(|e| e.into_inner());
        extensions.get(extension).cloned().unwrap_or_default()
    }

    /// Names of the extensions that have an entry.
    pub fn extensions(&self) -> Vec<String> {
        self.extensions.read().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::provider::tests::manifest;
    use crate::provider::{Backend, StubBackend, providers_for};

    use super::*;

    fn providers(name: &str, keywords: &str) -> Vec<Arc<dyn Provider>> {
        providers_for(&manifest(name, keywords), &(Arc::new(StubBackend) as Arc<dyn Backend>))
    }

    fn ids(registry: &Registry) -> Vec<String> {
        registry.providers().iter().map(|p| p.id().to_owned()).collect()
    }

    #[test]
    fn providers_of_all_extensions_are_listed_in_name_order() {
        let registry = Registry::new();
        registry.replace("zeta", providers("zeta", r#"["z"]"#));
        registry.replace("alpha", providers("alpha", r#"["a"]"#));
        assert_eq!(ids(&registry), ["alpha.search", "alpha.open", "zeta.search", "zeta.open"]);
        assert_eq!(registry.extensions(), ["alpha", "zeta"]);
    }

    #[test]
    fn replace_swaps_the_providers_of_one_extension() {
        let registry = Registry::new();
        registry.replace("gh", providers("gh", r#"["gh"]"#));
        registry.replace("other", providers("other", r#"["o"]"#));
        registry.replace("gh", providers("gh", r#"["git"]"#));
        let gh = registry.providers_of("gh");
        assert_eq!(gh.len(), 2);
        assert_eq!(gh[0].keywords(), ["git"]);
        assert_eq!(registry.providers().len(), 4);
    }

    #[test]
    fn remove_drops_only_that_extension() {
        let registry = Registry::new();
        registry.replace("gh", providers("gh", r#"["gh"]"#));
        registry.replace("other", providers("other", r#"["o"]"#));
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
}
