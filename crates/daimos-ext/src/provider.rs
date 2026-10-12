//! A manifest command as a [`Provider`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use daimos_core::model::{Action, ActionKind, Item, ItemBatch};
use daimos_core::provider::{ActionEffect, ActionOutcome, Provider, QueryContext};
use futures::StreamExt;
use futures::stream::{self, BoxStream};

use crate::builtin::{Opener, SystemOpener};
use crate::dir::LoadedExtension;
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

    /// Runs `action` of `item` when it is not a built-in kind; the answer is the toast to show, if
    /// any. May block until the extension answers or its timeout ends. The default cannot.
    fn custom_action(
        &self,
        _extension: &str,
        _command: &str,
        _item: &str,
        _action: &str,
    ) -> Result<Option<String>, String> {
        Err("this extension cannot run custom actions yet".into())
    }

    /// Called when an extension has been loaded or reloaded, before its providers are registered.
    fn load(&self, _extension: &LoadedExtension) {}

    /// Called when the extension called `name` is gone, disabled or about to be reloaded.
    fn unload(&self, _name: &str) {}
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
    opener: Arc<dyn Opener>,
    /// The actions of the items this command answered with, by item id.
    known: Arc<Mutex<HashMap<String, Vec<Action>>>>,
}

/// Items remembered for their actions; the oldest are dropped wholesale beyond this.
const KNOWN_LIMIT: usize = 2000;

impl CommandProvider {
    pub fn new(manifest: &Manifest, command: &Command, backend: Arc<dyn Backend>) -> Self {
        Self {
            id: format!("{}.{}", manifest.extension.name, command.name),
            extension: manifest.extension.name.clone(),
            label: manifest.extension.title.clone(),
            command: command.name.clone(),
            keywords: command.keywords.clone(),
            backend,
            opener: Arc::new(SystemOpener),
            known: Arc::default(),
        }
    }

    /// Replaces the opener that runs the built-in `open` and `run` actions.
    #[must_use]
    pub fn with_opener(mut self, opener: Arc<dyn Opener>) -> Self {
        self.opener = opener;
        self
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
        let known = Arc::clone(&self.known);
        self.backend
            .query(&self.extension, &self.command, ctx)
            .map(move |mut batch| {
                {
                    let mut known = known.lock().unwrap_or_else(|e| e.into_inner());
                    if known.len() > KNOWN_LIMIT {
                        known.clear();
                    }
                    for item in &batch.items {
                        known.insert(item.id.clone(), item.actions.clone());
                    }
                }
                for item in &mut batch.items {
                    if !item.accessories.contains(&label) {
                        item.accessories.push(label.clone());
                    }
                }
                batch
            })
            .boxed()
    }

    fn run_action_outcome(
        &self,
        item_id: &str,
        action_id: &str,
    ) -> Option<Result<ActionOutcome, String>> {
        let action = {
            let known = self.known.lock().unwrap_or_else(|e| e.into_inner());
            let actions = known.get(item_id)?;
            actions.iter().find(|action| action.id == action_id).cloned()
        };
        let Some(action) = action else {
            return Some(Err(format!("no action {action_id}")));
        };
        let payload = action.payload.as_deref().unwrap_or_default();
        let hide = |result: std::io::Result<()>| {
            result
                .map(|()| ActionOutcome::from(ActionEffect::Hide))
                .map_err(|error| format!("{}: {error}", action.title))
        };
        Some(match action.kind {
            // Built-in kinds are the daimos' job: the script never starts for them.
            ActionKind::Open => hide(self.opener.open(payload)),
            ActionKind::Run => hide(self.opener.run(payload)),
            ActionKind::Custom => self
                .backend
                .custom_action(&self.extension, &self.command, item_id, action_id)
                .map(|toast| ActionOutcome { effect: ActionEffect::Stay, toast }),
            ActionKind::Copy | ActionKind::Paste => {
                Err("copy and paste run in the window, not in the host".into())
            }
            ActionKind::Push | ActionKind::Form => Err("extensions cannot push views yet".into()),
        })
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

    /// Records what the host was asked to open or run.
    #[derive(Default)]
    pub(crate) struct Recorder {
        pub(crate) calls: Mutex<Vec<String>>,
        pub(crate) fail: bool,
    }

    impl Opener for Recorder {
        fn open(&self, target: &str) -> std::io::Result<()> {
            self.calls.lock().unwrap().push(format!("open {target}"));
            if self.fail { Err(std::io::Error::other("no browser")) } else { Ok(()) }
        }
        fn run(&self, command_line: &str) -> std::io::Result<()> {
            self.calls.lock().unwrap().push(format!("run {command_line}"));
            Ok(())
        }
    }

    /// Answers with one item that has an action of every kind, and counts custom actions.
    #[derive(Default)]
    struct WithActions {
        custom: Mutex<Vec<String>>,
    }

    impl Backend for WithActions {
        fn query(&self, _: &str, _: &str, _: QueryContext) -> BoxStream<'static, ItemBatch> {
            let mut item = Item::new("r1", "Repo");
            let action = |id: &str, kind, payload: Option<&str>| Action {
                id: id.into(),
                title: id.into(),
                shortcut: None,
                kind,
                payload: payload.map(Into::into),
            };
            item.actions = vec![
                action("open", ActionKind::Open, Some("https://example.com")),
                action("term", ActionKind::Run, Some("git status")),
                action("copy", ActionKind::Copy, Some("text")),
                action("star", ActionKind::Custom, None),
                action("view", ActionKind::Push, Some("v")),
            ];
            stream::once(async move { ItemBatch::from(vec![item]) }).boxed()
        }

        fn custom_action(
            &self,
            ext: &str,
            command: &str,
            item: &str,
            action: &str,
        ) -> Result<Option<String>, String> {
            self.custom.lock().unwrap().push(format!("{ext}/{command}/{item}/{action}"));
            if item == "r1" { Ok(Some("Starred".into())) } else { Err("nope".into()) }
        }
    }

    fn provider_with(recorder: &Arc<Recorder>) -> (CommandProvider, Arc<WithActions>) {
        let backend = Arc::new(WithActions::default());
        let manifest = manifest("gh", r#"["gh"]"#);
        let provider = CommandProvider::new(&manifest, &manifest.commands[0], backend.clone())
            .with_opener(recorder.clone());
        futures::executor::block_on(provider.query(ctx("x")).collect::<Vec<_>>());
        (provider, backend)
    }

    #[test]
    fn built_in_actions_run_in_the_host_and_never_reach_the_backend() {
        let recorder = Arc::new(Recorder::default());
        let (provider, backend) = provider_with(&recorder);
        let hide = Some(Ok(ActionOutcome { effect: ActionEffect::Hide, toast: None }));
        assert_eq!(provider.run_action_outcome("r1", "open"), hide);
        assert_eq!(provider.run_action_outcome("r1", "term"), hide);
        assert_eq!(*recorder.calls.lock().unwrap(), ["open https://example.com", "run git status"]);
        assert!(backend.custom.lock().unwrap().is_empty());
    }

    #[test]
    fn a_custom_action_goes_to_the_backend_and_its_toast_comes_back() {
        let recorder = Arc::new(Recorder::default());
        let (provider, backend) = provider_with(&recorder);
        assert_eq!(
            provider.run_action_outcome("r1", "star"),
            Some(Ok(ActionOutcome { effect: ActionEffect::Stay, toast: Some("Starred".into()) }))
        );
        assert_eq!(*backend.custom.lock().unwrap(), ["gh/search/r1/star"]);
        assert!(recorder.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn failures_and_unsupported_kinds_are_errors() {
        let recorder = Arc::new(Recorder { fail: true, ..Recorder::default() });
        let (provider, _) = provider_with(&recorder);
        let error = provider.run_action_outcome("r1", "open").unwrap().unwrap_err();
        assert!(error.contains("no browser"), "{error}");
        for action in ["copy", "view", "missing"] {
            assert!(provider.run_action_outcome("r1", action).unwrap().is_err(), "{action}");
        }
    }

    #[test]
    fn items_of_other_providers_are_not_ours() {
        let recorder = Arc::new(Recorder::default());
        let (provider, _) = provider_with(&recorder);
        assert_eq!(provider.run_action_outcome("app:firefox", "open"), None);
    }

    #[test]
    fn the_default_backend_cannot_run_custom_actions() {
        assert!(StubBackend.custom_action("a", "b", "c", "d").is_err());
    }
}
