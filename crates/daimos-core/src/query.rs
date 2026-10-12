//! The query engine: fans a query out to the providers, streams batches, cancels per session.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::runtime::Handle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::model::Item;
use crate::provider::{Provider, QueryContext};

/// Default time per provider and query.
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(100);

/// One event for the UI. `seq` increases per query; the last batch of a provider has `done`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ResultBatch {
    pub session: u32,
    pub seq: u32,
    pub provider: String,
    pub items: Vec<Item>,
    pub done: bool,
    /// The provider's weight; the UI lists providers with a higher weight first.
    pub weight: f32,
}

pub type Sink = Arc<dyn Fn(ResultBatch) + Send + Sync>;

struct Running {
    cancel: CancellationToken,
    seq: AtomicU32,
}

/// What the user chose in the settings; it applies to the next query, so changes are live.
#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    /// Providers that are switched off, by id.
    pub disabled: HashSet<String>,
    /// Replaces a provider's own weight (the order among providers), by id.
    pub weights: HashMap<String, f32>,
    /// Lowercase alias to the text searched instead when the whole query is the alias.
    pub aliases: HashMap<String, String>,
}

impl From<&crate::settings::Settings> for EngineConfig {
    fn from(settings: &crate::settings::Settings) -> Self {
        let mut config = Self::default();
        for (id, provider) in &settings.providers {
            if !provider.enabled {
                config.disabled.insert(id.clone());
            }
            if let Some(weight) = provider.weight {
                config.weights.insert(id.clone(), weight);
            }
        }
        config.aliases = settings
            .aliases
            .iter()
            .filter(|(alias, target)| !alias.trim().is_empty() && !target.trim().is_empty())
            .map(|(alias, target)| (alias.trim().to_lowercase(), target.trim().to_owned()))
            .collect();
        config
    }
}

pub struct QueryEngine {
    providers: Vec<Arc<dyn Provider>>,
    sink: Sink,
    handle: Handle,
    sessions: Mutex<HashMap<u32, Arc<Running>>>,
    config: RwLock<EngineConfig>,
}

impl QueryEngine {
    /// Providers run on the runtime behind `handle`; batches go to `sink`.
    pub fn new(providers: Vec<Arc<dyn Provider>>, sink: Sink, handle: Handle) -> Self {
        Self {
            providers,
            sink,
            handle,
            sessions: Mutex::new(HashMap::new()),
            config: RwLock::new(EngineConfig::default()),
        }
    }

    /// Replaces the user's choices; the next query uses them.
    pub fn set_config(&self, config: EngineConfig) {
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = config;
    }

    /// The ids and own weights of all providers, for the settings page.
    pub fn providers(&self) -> Vec<(String, f32)> {
        self.providers.iter().map(|p| (p.id().to_owned(), p.weight())).collect()
    }

    /// Cancels the previous query of `session` and starts a new one.
    pub fn query(&self, session: u32, text: &str) {
        let running =
            Arc::new(Running { cancel: CancellationToken::new(), seq: AtomicU32::new(0) });
        let previous = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session, Arc::clone(&running));
        if let Some(previous) = previous {
            previous.cancel.cancel();
        }

        let config = self.config.read().unwrap_or_else(|e| e.into_inner()).clone();
        // An alias is the whole query: `ff` searches for what it stands for.
        let text = config.aliases.get(&text.trim().to_lowercase()).map_or(text, String::as_str);
        let (targets, text, keyword_routed) = self.route(text);
        for provider in targets.into_iter().filter(|p| !config.disabled.contains(p.id())) {
            let weight =
                config.weights.get(provider.id()).copied().unwrap_or_else(|| provider.weight());
            let ctx = QueryContext {
                text: text.clone(),
                keyword_routed,
                cancel: running.cancel.child_token(),
            };
            self.handle.spawn(run_provider(
                provider,
                weight,
                ctx,
                session,
                Arc::clone(&running),
                Arc::clone(&self.sink),
            ));
        }
    }

    /// The view `view_id` of `provider`, for a `push` or `form` action.
    pub fn view(&self, provider: &str, view_id: &str) -> Option<crate::model::View> {
        self.providers.iter().find(|p| p.id() == provider)?.view(view_id)
    }

    /// Lets the providers run an action of an item; `None` when no provider owns the item.
    pub fn run_action(
        &self,
        item_id: &str,
        action_id: &str,
    ) -> Option<Result<crate::provider::ActionEffect, String>> {
        self.providers.iter().find_map(|p| p.run_action(item_id, action_id))
    }

    /// Hands the values of a submitted form to its provider.
    pub fn submit(
        &self,
        provider: &str,
        view_id: &str,
        values: &HashMap<String, String>,
    ) -> Result<(), String> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == provider)
            .ok_or_else(|| format!("no provider {provider}"))?;
        provider.submit(view_id, values)
    }

    /// Stops the query of a session, for example when the window hides.
    pub fn cancel(&self, session: u32) {
        let running = self.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&session);
        if let Some(running) = running {
            running.cancel.cancel();
        }
    }

    /// `gh repo` goes to the provider with keyword `gh` only; everything else to all providers.
    fn route(&self, text: &str) -> (Vec<Arc<dyn Provider>>, String, bool) {
        let trimmed = text.trim_start();
        let (first, rest) = trimmed.split_once(char::is_whitespace).unwrap_or((trimmed, ""));
        if !first.is_empty() && trimmed.len() > first.len() {
            let keyword = first.to_lowercase();
            let routed: Vec<_> = self
                .providers
                .iter()
                .filter(|p| p.keywords().iter().any(|k| k.to_lowercase() == keyword))
                .cloned()
                .collect();
            if !routed.is_empty() {
                return (routed, rest.trim_start().to_owned(), true);
            }
        }
        (self.providers.clone(), text.to_owned(), false)
    }
}

async fn run_provider(
    provider: Arc<dyn Provider>,
    weight: f32,
    ctx: QueryContext,
    session: u32,
    running: Arc<Running>,
    sink: Sink,
) {
    let id = provider.id().to_owned();
    let cancel = ctx.cancel.clone();
    let deadline = Instant::now() + provider.budget();
    let mut stream = provider.query(ctx);

    let emit = |items: Vec<Item>, done: bool| {
        // A cancelled query never reports, so a stale batch cannot overwrite newer results.
        if running.cancel.is_cancelled() {
            return;
        }
        let seq = running.seq.fetch_add(1, Ordering::SeqCst);
        sink(ResultBatch { session, seq, provider: id.clone(), items, done, weight });
    };

    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep_until(deadline) => {
                log::warn!("provider {id} exceeded its budget of {:?}; keeping partial results", provider.budget());
                cancel.cancel();
                break;
            }
            batch = stream.next() => match batch {
                Some(batch) => emit(batch.items, false),
                None => break,
            },
        }
    }
    // The budget cut-off cancels only the child token, so the query itself still reports.
    emit_done(&running, &sink, session, &id, weight);
}

fn emit_done(running: &Running, sink: &Sink, session: u32, provider: &str, weight: f32) {
    if running.cancel.is_cancelled() {
        return;
    }
    let seq = running.seq.fetch_add(1, Ordering::SeqCst);
    sink(ResultBatch {
        session,
        seq,
        provider: provider.to_owned(),
        items: Vec::new(),
        done: true,
        weight,
    });
}

#[cfg(test)]
mod tests {
    use futures::stream::{self, BoxStream};
    use tokio::sync::mpsc;

    use super::*;
    use crate::model::ItemBatch;

    /// Emits `batches` with `delay` before each one.
    struct Fake {
        id: &'static str,
        keyword: Option<&'static str>,
        batches: Vec<Vec<&'static str>>,
        delay: Duration,
        budget: Duration,
    }

    impl Fake {
        fn new(id: &'static str, batches: Vec<Vec<&'static str>>) -> Self {
            Self { id, keyword: None, batches, delay: Duration::ZERO, budget: DEFAULT_BUDGET }
        }
    }

    impl Provider for Fake {
        fn id(&self) -> &str {
            self.id
        }
        fn keyword(&self) -> Option<&str> {
            self.keyword
        }
        fn budget(&self) -> Duration {
            self.budget
        }
        fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
            let delay = self.delay;
            let id = self.id;
            let batches = self.batches.clone();
            let text = ctx.text;
            stream::iter(batches)
                .then(move |ids| {
                    let text = text.clone();
                    async move {
                        tokio::time::sleep(delay).await;
                        ItemBatch::from(
                            ids.into_iter()
                                .map(|i| Item::new(format!("{id}:{i}"), text.clone()))
                                .collect::<Vec<_>>(),
                        )
                    }
                })
                .boxed()
        }
    }

    fn engine(providers: Vec<Fake>) -> (QueryEngine, mpsc::UnboundedReceiver<ResultBatch>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink: Sink = Arc::new(move |b| {
            let _ = tx.send(b);
        });
        let providers = providers.into_iter().map(|p| Arc::new(p) as Arc<dyn Provider>).collect();
        (QueryEngine::new(providers, sink, Handle::current()), rx)
    }

    /// Collects batches until every named provider reported `done`.
    async fn until_done(
        rx: &mut mpsc::UnboundedReceiver<ResultBatch>,
        providers: usize,
    ) -> Vec<ResultBatch> {
        let mut out = Vec::new();
        let mut done = 0;
        while done < providers {
            let b = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
            done += usize::from(b.done);
            out.push(b);
        }
        out
    }

    #[tokio::test]
    async fn streams_batches_in_order_and_marks_done() {
        let (engine, mut rx) = engine(vec![Fake::new("a", vec![vec!["1"], vec!["2", "3"]])]);
        engine.query(7, "x");
        let batches = until_done(&mut rx, 1).await;
        assert_eq!(batches.iter().map(|b| b.seq).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(batches.iter().all(|b| b.session == 7 && b.provider == "a"));
        assert_eq!(batches[1].items.len(), 2);
        assert!(batches[2].done && batches[2].items.is_empty());
        assert!(!batches[0].done);
    }

    #[tokio::test]
    async fn a_slow_provider_does_not_delay_a_fast_one() {
        let mut slow = Fake::new("slow", vec![vec!["s"]]);
        slow.delay = Duration::from_millis(60);
        let (engine, mut rx) = engine(vec![slow, Fake::new("fast", vec![vec!["f"]])]);
        engine.query(1, "x");
        let first = rx.recv().await.unwrap();
        assert_eq!(first.provider, "fast");
    }

    #[tokio::test]
    async fn a_new_query_cancels_the_previous_one_of_the_session() {
        let mut slow = Fake::new("slow", vec![vec!["old"]]);
        slow.delay = Duration::from_millis(80);
        slow.budget = Duration::from_secs(1);
        let (engine, mut rx) = engine(vec![slow]);
        engine.query(1, "first");
        tokio::time::sleep(Duration::from_millis(10)).await;
        engine.query(1, "second");
        let batches = until_done(&mut rx, 1).await;
        assert!(batches.iter().all(|b| b.items.iter().all(|i| i.title == "second")));
        // Nothing of the cancelled query arrives afterwards.
        assert!(tokio::time::timeout(Duration::from_millis(150), rx.recv()).await.is_err());
    }

    #[tokio::test]
    async fn sessions_do_not_cancel_each_other() {
        let mut slow = Fake::new("slow", vec![vec!["x"]]);
        slow.delay = Duration::from_millis(30);
        let (engine, mut rx) = engine(vec![slow]);
        engine.query(1, "a");
        engine.query(2, "b");
        let batches = until_done(&mut rx, 2).await;
        let mut sessions: Vec<_> = batches.iter().filter(|b| b.done).map(|b| b.session).collect();
        sessions.sort_unstable();
        assert_eq!(sessions, [1, 2]);
    }

    #[tokio::test]
    async fn a_provider_over_budget_keeps_partial_results() {
        let mut p = Fake::new("late", vec![vec!["early"], vec!["late"]]);
        p.delay = Duration::from_millis(60);
        p.budget = Duration::from_millis(100);
        let (engine, mut rx) = engine(vec![p]);
        engine.query(1, "x");
        let batches = until_done(&mut rx, 1).await;
        let ids: Vec<_> =
            batches.iter().flat_map(|b| b.items.iter().map(|i| i.id.as_str())).collect();
        assert_eq!(ids, ["late:early"]);
        assert!(batches.last().unwrap().done);
    }

    #[tokio::test]
    async fn a_keyword_routes_to_that_provider_only() {
        let mut gh = Fake::new("gh", vec![vec!["repo"]]);
        gh.keyword = Some("gh");
        let (engine, mut rx) = engine(vec![gh, Fake::new("apps", vec![vec!["a"]])]);
        engine.query(1, "gh my repo");
        let batches = until_done(&mut rx, 1).await;
        assert!(batches.iter().all(|b| b.provider == "gh"));
        assert_eq!(batches[0].items[0].title, "my repo");
        assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv()).await.is_err());
    }

    /// A provider that answers to several keywords.
    struct MultiKeyword;

    impl Provider for MultiKeyword {
        fn id(&self) -> &str {
            "multi"
        }
        fn keywords(&self) -> Vec<&str> {
            vec!["gh", "repo"]
        }
        fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
            stream::once(async move { ItemBatch::from(vec![Item::new("m", ctx.text)]) }).boxed()
        }
    }

    #[tokio::test]
    async fn every_keyword_of_a_provider_routes_to_it() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink: Sink = Arc::new(move |b| {
            let _ = tx.send(b);
        });
        let providers: Vec<Arc<dyn Provider>> =
            vec![Arc::new(MultiKeyword), Arc::new(Fake::new("apps", vec![vec!["a"]]))];
        let engine = QueryEngine::new(providers, sink, Handle::current());
        for keyword in ["gh", "REPO"] {
            engine.query(1, &format!("{keyword} thing"));
            let batches = until_done(&mut rx, 1).await;
            assert!(batches.iter().all(|b| b.provider == "multi"), "{keyword}");
            assert_eq!(batches[0].items[0].title, "thing");
        }
    }

    #[tokio::test]
    async fn a_lone_keyword_or_an_unknown_prefix_goes_to_every_provider() {
        let mut gh = Fake::new("gh", vec![vec!["r"]]);
        gh.keyword = Some("gh");
        let (engine, mut rx) = engine(vec![gh, Fake::new("apps", vec![vec!["a"]])]);
        engine.query(1, "gh");
        assert_eq!(until_done(&mut rx, 2).await.iter().filter(|b| b.done).count(), 2);
        engine.query(1, "zz thing");
        assert_eq!(until_done(&mut rx, 2).await.iter().filter(|b| b.done).count(), 2);
    }

    /// Everything the provider got as text, so a test can see what was searched.
    struct Echo(&'static str);

    impl Provider for Echo {
        fn id(&self) -> &str {
            self.0
        }
        fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
            stream::once(async move { ItemBatch::from(vec![Item::new("e", ctx.text)]) }).boxed()
        }
    }

    #[tokio::test]
    async fn a_disabled_provider_is_skipped_and_a_weight_overrides_the_own_one() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink: Sink = Arc::new(move |b| {
            let _ = tx.send(b);
        });
        let engine = QueryEngine::new(
            vec![Arc::new(Echo("one")), Arc::new(Echo("two")), Arc::new(Echo("three"))],
            sink,
            Handle::current(),
        );
        engine.set_config(EngineConfig {
            disabled: HashSet::from(["two".to_owned()]),
            weights: HashMap::from([("three".to_owned(), 7.0)]),
            aliases: HashMap::new(),
        });
        engine.query(1, "x");
        let batches = until_done(&mut rx, 2).await;
        let mut providers: Vec<_> =
            batches.iter().map(|b| (b.provider.as_str(), b.weight)).collect();
        providers.sort_by(|a, b| a.0.cmp(b.0));
        providers.dedup();
        assert_eq!(providers, [("one", 1.0), ("three", 7.0)]);
        assert!(tokio::time::timeout(Duration::from_millis(100), rx.recv()).await.is_err());

        // Changes apply to the next query.
        engine.set_config(EngineConfig::default());
        engine.query(1, "y");
        assert_eq!(until_done(&mut rx, 3).await.iter().filter(|b| b.done).count(), 3);
    }

    #[tokio::test]
    async fn an_alias_replaces_the_whole_query_only() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink: Sink = Arc::new(move |b| {
            let _ = tx.send(b);
        });
        let engine = QueryEngine::new(vec![Arc::new(Echo("echo"))], sink, Handle::current());
        engine.set_config(EngineConfig {
            aliases: HashMap::from([("ff".to_owned(), "Firefox".to_owned())]),
            ..EngineConfig::default()
        });
        let searched = |batches: Vec<ResultBatch>| {
            batches.into_iter().find_map(|b| b.items.first().map(|i| i.title.clone()))
        };
        engine.query(1, " FF ");
        assert_eq!(searched(until_done(&mut rx, 1).await).as_deref(), Some("Firefox"));
        engine.query(1, "ff x");
        assert_eq!(searched(until_done(&mut rx, 1).await).as_deref(), Some("ff x"));
        engine.query(1, "f");
        assert_eq!(searched(until_done(&mut rx, 1).await).as_deref(), Some("f"));
    }

    #[test]
    fn settings_become_an_engine_config() {
        use crate::settings::{ProviderSetting, Settings};
        let settings = Settings {
            providers: [
                ("calc".to_owned(), ProviderSetting { enabled: false, weight: Some(2.0) }),
                ("apps".to_owned(), ProviderSetting { enabled: true, weight: Some(5.0) }),
                ("other".to_owned(), ProviderSetting::default()),
            ]
            .into(),
            aliases: [
                (" FF ".to_owned(), " Firefox ".to_owned()),
                (String::new(), "x".to_owned()),
                ("empty".to_owned(), "  ".to_owned()),
            ]
            .into(),
            ..Settings::default()
        };
        let config = EngineConfig::from(&settings);
        assert_eq!(config.disabled, HashSet::from(["calc".to_owned()]));
        assert_eq!(
            config.weights,
            HashMap::from([("calc".to_owned(), 2.0), ("apps".to_owned(), 5.0)])
        );
        assert_eq!(config.aliases, HashMap::from([("ff".to_owned(), "Firefox".to_owned())]));
    }

    #[test]
    fn the_engine_lists_its_providers_with_their_weights() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let sink: Sink = Arc::new(|_| {});
        let engine = QueryEngine::new(
            vec![Arc::new(Echo("a")), Arc::new(WithViews)],
            sink,
            rt.handle().clone(),
        );
        assert_eq!(engine.providers(), [("a".to_owned(), 1.0), ("views".to_owned(), 1.0)]);
    }

    /// A provider with one list view and a form that accepts a value of `name` only.
    struct WithViews;

    impl Provider for WithViews {
        fn id(&self) -> &str {
            "views"
        }
        fn query(&self, _: QueryContext) -> BoxStream<'static, ItemBatch> {
            stream::empty().boxed()
        }
        fn view(&self, view_id: &str) -> Option<crate::model::View> {
            (view_id == "list")
                .then(|| crate::model::View::List { items: vec![Item::new("x", "X")] })
        }
        fn submit(&self, view_id: &str, values: &HashMap<String, String>) -> Result<(), String> {
            match (view_id, values.get("name")) {
                ("form", Some(name)) if !name.is_empty() => Ok(()),
                _ => Err("name is required".into()),
            }
        }
    }

    #[tokio::test]
    async fn views_and_forms_are_routed_to_their_provider() {
        let (tx, _rx) = mpsc::unbounded_channel::<ResultBatch>();
        let sink: Sink = Arc::new(move |b| {
            let _ = tx.send(b);
        });
        let engine = QueryEngine::new(
            vec![Arc::new(WithViews), Arc::new(Fake::new("plain", vec![]))],
            sink,
            Handle::current(),
        );
        assert!(matches!(engine.view("views", "list"), Some(crate::model::View::List { .. })));
        assert!(engine.view("views", "missing").is_none());
        assert!(engine.view("plain", "list").is_none());
        assert!(engine.view("nobody", "list").is_none());

        let name = |v: &str| HashMap::from([("name".to_owned(), v.to_owned())]);
        assert_eq!(engine.submit("views", "form", &name("Ada")), Ok(()));
        assert_eq!(engine.submit("views", "form", &name("")), Err("name is required".into()));
        assert!(engine.submit("plain", "form", &name("Ada")).is_err());
        assert!(engine.submit("nobody", "form", &name("Ada")).unwrap_err().contains("no provider"));
    }

    #[tokio::test]
    async fn cancel_silences_the_session() {
        let mut slow = Fake::new("slow", vec![vec!["x"]]);
        slow.delay = Duration::from_millis(50);
        let (engine, mut rx) = engine(vec![slow]);
        engine.query(1, "a");
        engine.cancel(1);
        assert!(tokio::time::timeout(Duration::from_millis(150), rx.recv()).await.is_err());
    }
}
