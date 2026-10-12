//! A manifest command as a [`Provider`].

use std::sync::Arc;

use daimos_core::model::{Item, ItemBatch};
use daimos_core::provider::{Provider, QueryContext};
use futures::StreamExt;
use futures::stream::{self, BoxStream};

use crate::manifest::{Command, Manifest};

/// Runs the commands of extensions. The script and wasm hosts implement it in later milestones.
pub trait Backend: Send + Sync {
    /// Answers a query of `command` of `extension`.
    fn query(
        &self,
        extension: &str,
        command: &str,
        ctx: QueryContext,
    ) -> BoxStream<'static, ItemBatch>;
}

/// Answers every query with nothing, until a runtime host exists.
#[derive(Debug, Clone, Copy, Default)]
pub struct StubBackend;

impl Backend for StubBackend {
    fn query(&self, _: &str, _: &str, _: QueryContext) -> BoxStream<'static, ItemBatch> {
        stream::empty().boxed()
    }
}

/// One command of one extension.
pub struct CommandProvider {
    id: String,
    extension: String,
    /// The extension's title, shown on its items.
    label: String,
    command: String,
    keywords: Vec<String>,
    backend: Arc<dyn Backend>,
}

impl CommandProvider {
    pub fn new(manifest: &Manifest, command: &Command, backend: Arc<dyn Backend>) -> Self {
        Self {
            id: format!("{}.{}", manifest.extension.name, command.name),
            extension: manifest.extension.name.clone(),
            label: manifest.extension.title.clone(),
            command: command.name.clone(),
            keywords: command.keywords.clone(),
            backend,
        }
    }
}

impl Provider for CommandProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn keyword(&self) -> Option<&str> {
        self.keywords.first().map(String::as_str)
    }

    fn keywords(&self) -> Vec<&str> {
        self.keywords.iter().map(String::as_str).collect()
    }

    fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
        let label = self.label.clone();
        self.backend
            .query(&self.extension, &self.command, ctx)
            .map(move |mut batch| {
                for item in &mut batch.items {
                    if !item.accessories.contains(&label) {
                        item.accessories.push(label.clone());
                    }
                }
                batch
            })
            .boxed()
    }
}

/// The shape of an error item: the problem as the title, the message under it, and the extension
/// it concerns as the label.
pub fn error_item(id: String, title: String, message: &str, label: &str) -> Item {
    let mut item = Item::new(id, title);
    item.subtitle = Some(message.to_owned());
    item.accessories.push(label.to_owned());
    item
}

/// The one item a broken extension shows instead of its commands.
///
/// It appears for an empty query and for a query that names the extension's directory, so the
/// problem is seen without cluttering every search.
pub struct ErrorProvider {
    id: String,
    key: String,
    message: String,
}

impl ErrorProvider {
    pub fn new(key: &str, message: &str) -> Self {
        Self { id: format!("{key}.error"), key: key.to_owned(), message: message.to_owned() }
    }

    fn matches(&self, text: &str) -> bool {
        let text = text.trim().to_lowercase();
        let key = self.key.to_lowercase();
        text.is_empty() || key.contains(&text) || text.contains(&key)
    }
}

impl Provider for ErrorProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
        if !self.matches(&ctx.text) {
            return stream::empty().boxed();
        }
        let item = error_item(
            self.id.clone(),
            format!("{} could not be loaded", self.key),
            &self.message,
            &self.key,
        );
        stream::once(async move { ItemBatch::from(vec![item]) }).boxed()
    }
}

/// One provider per command of `manifest`, in manifest order.
pub fn providers_for(manifest: &Manifest, backend: &Arc<dyn Backend>) -> Vec<Arc<dyn Provider>> {
    manifest
        .commands
        .iter()
        .map(|command| {
            Arc::new(CommandProvider::new(manifest, command, Arc::clone(backend)))
                as Arc<dyn Provider>
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use tokio_util::sync::CancellationToken;

    use super::*;

    pub(crate) fn manifest(name: &str, keywords: &str) -> Manifest {
        let source = format!(
            "[extension]\nname = \"{name}\"\ntitle = \"Title of {name}\"\nversion = \"1.0.0\"\n\
             runtime = \"script\"\n\n\
             [[commands]]\nname = \"search\"\ntitle = \"Search\"\nkeywords = {keywords}\n\n\
             [[commands]]\nname = \"open\"\ntitle = \"Open\"\n\n\
             [runtime.script]\ncommand = [\"run\"]\n"
        );
        Manifest::parse(&source, Path::new("extension.toml")).unwrap()
    }

    pub(crate) fn ctx(text: &str) -> QueryContext {
        QueryContext { text: text.into(), keyword_routed: true, cancel: CancellationToken::new() }
    }

    /// Answers with one item that echoes the extension, command and text.
    struct Echo;

    impl Backend for Echo {
        fn query(
            &self,
            ext: &str,
            command: &str,
            ctx: QueryContext,
        ) -> BoxStream<'static, ItemBatch> {
            let item = Item::new(format!("{ext}/{command}"), ctx.text);
            stream::once(async move { ItemBatch::from(vec![item]) }).boxed()
        }
    }

    #[test]
    fn each_command_becomes_a_provider_with_the_manifest_keywords() {
        let manifest = manifest("gh", r#"["gh", "repo"]"#);
        let backend: Arc<dyn Backend> = Arc::new(StubBackend);
        let providers = providers_for(&manifest, &backend);
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].id(), "gh.search");
        assert_eq!(providers[0].keyword(), Some("gh"));
        assert_eq!(providers[0].keywords(), ["gh", "repo"]);
        assert_eq!(providers[1].id(), "gh.open");
        assert_eq!(providers[1].keyword(), None);
        assert!(providers[1].keywords().is_empty());
    }

    #[test]
    fn the_stub_backend_yields_nothing() {
        let manifest = manifest("gh", r#"["gh"]"#);
        let providers = providers_for(&manifest, &(Arc::new(StubBackend) as Arc<dyn Backend>));
        let batches = futures::executor::block_on(providers[0].query(ctx("x")).collect::<Vec<_>>());
        assert!(batches.is_empty());
    }

    #[test]
    fn items_are_labeled_with_the_extension_once() {
        let manifest = manifest("gh", r#"["gh"]"#);
        let providers = providers_for(&manifest, &(Arc::new(Echo) as Arc<dyn Backend>));
        let batches =
            futures::executor::block_on(providers[0].query(ctx("lau")).collect::<Vec<_>>());
        assert_eq!(batches.len(), 1);
        let item = &batches[0].items[0];
        assert_eq!((item.id.as_str(), item.title.as_str()), ("gh/search", "lau"));
        assert_eq!(item.accessories, ["Title of gh"]);
    }

    #[test]
    fn the_error_provider_shows_one_item_with_the_message() {
        let provider = ErrorProvider::new("github", "extension.toml:3: extension.name: bad");
        assert_eq!(provider.id(), "github.error");
        for text in ["", "  ", "git", "github", "github foo"] {
            let batches =
                futures::executor::block_on(provider.query(ctx(text)).collect::<Vec<_>>());
            assert_eq!(batches.len(), 1, "{text:?}");
            let item = &batches[0].items[0];
            assert_eq!(item.subtitle.as_deref(), Some("extension.toml:3: extension.name: bad"));
        }
    }

    #[test]
    fn the_error_provider_stays_quiet_for_unrelated_queries() {
        let provider = ErrorProvider::new("github", "bad");
        let batches =
            futures::executor::block_on(provider.query(ctx("firefox")).collect::<Vec<_>>());
        assert!(batches.is_empty());
    }
}
