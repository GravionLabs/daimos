//! The provider behind the application list: the index snapshot, matched and ranked per query.

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use futures::stream::{self, BoxStream};

use crate::apps::AppEntry;
use crate::frecency::Frecency;
use crate::index::IndexService;
use crate::model::{Action, ActionKind, Detail, Icon, Item, ItemBatch};
use crate::provider::{Provider, QueryContext};
use crate::ranker::{Candidate, Ranker};

/// Most results one query returns.
const LIMIT: usize = 50;

pub struct AppsProvider {
    index: Arc<IndexService>,
    frecency: Arc<Frecency>,
    ranker: Mutex<Ranker>,
}

impl AppsProvider {
    pub fn new(index: Arc<IndexService>, frecency: Arc<Frecency>) -> Self {
        Self { index, frecency, ranker: Mutex::new(Ranker::new()) }
    }
}

/// The item for an application: `Enter` opens it, the source's own actions follow.
pub fn item_for(app: &AppEntry) -> Item {
    let mut item = Item::new(app.id.clone(), app.name.clone());
    item.subtitle = app.generic_name.clone();
    item.keywords = app.keywords.clone();
    item.icon = app.icon.as_ref().map(|p| Icon::File { path: p.to_string_lossy().into_owned() });
    item.actions = std::iter::once(Action {
        id: "open".into(),
        title: "Open".into(),
        shortcut: Some("Enter".into()),
        kind: ActionKind::Open,
        payload: Some(app.id.clone()),
    })
    .chain(app.actions.iter().map(|a| Action {
        id: a.id.clone(),
        title: a.name.clone(),
        shortcut: None,
        kind: ActionKind::Run,
        payload: Some(format!("{}#{}", app.id, a.id)),
    }))
    .chain(std::iter::once(Action {
        id: "copy".into(),
        title: "Copy Command".into(),
        shortcut: None,
        kind: ActionKind::Copy,
        payload: Some(app.exec.clone()),
    }))
    .collect();
    let mut entries = vec![("Command".to_owned(), app.exec.clone())];
    if let Some(generic) = &app.generic_name {
        entries.insert(0, ("Kind".to_owned(), generic.clone()));
    }
    if !app.keywords.is_empty() {
        entries.push(("Keywords".to_owned(), app.keywords.join(", ")));
    }
    item.detail = Some(Detail::Metadata { entries });
    item
}

impl Provider for AppsProvider {
    fn id(&self) -> &str {
        "apps"
    }

    fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
        let apps = self.index.apps();
        let items: Vec<Item> = apps.iter().map(item_for).collect();
        let candidates: Vec<_> =
            items.iter().map(|item| Candidate { item, weight: self.weight() }).collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        let ranked = self.ranker.lock().unwrap_or_else(|e| e.into_inner()).rank(
            &ctx.text,
            &candidates,
            &self.frecency,
            now,
            LIMIT,
        );
        let batch = ItemBatch::from(
            ranked
                .into_iter()
                .map(|r| Item { highlight: r.title_indices, ..items[r.index].clone() })
                .collect::<Vec<_>>(),
        );
        stream::once(async move { batch }).boxed()
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::apps::{AppAction, AppSource};
    use crate::index::AppIndex;

    struct Fixed(Vec<AppEntry>);

    impl AppSource for Fixed {
        fn id(&self) -> &str {
            "fake"
        }
        fn scan(&self) -> Vec<AppEntry> {
            self.0.clone()
        }
        fn watch_paths(&self) -> Vec<std::path::PathBuf> {
            vec![]
        }
        fn launch(&self, _: &AppEntry, _: Option<&str>) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn entry(id: &str, name: &str) -> AppEntry {
        AppEntry {
            id: id.into(),
            name: name.into(),
            generic_name: None,
            keywords: vec![],
            exec: name.into(),
            terminal: false,
            icon: None,
            actions: vec![],
        }
    }

    async fn run(provider: &AppsProvider, text: &str) -> Vec<Item> {
        let ctx = QueryContext {
            text: text.into(),
            keyword_routed: false,
            cancel: CancellationToken::new(),
        };
        provider.query(ctx).collect::<Vec<_>>().await.into_iter().flat_map(|b| b.items).collect()
    }

    fn provider(apps: Vec<AppEntry>, frecency: Arc<Frecency>) -> AppsProvider {
        let service =
            IndexService::open(AppIndex::in_memory().unwrap(), vec![Arc::new(Fixed(apps))], None)
                .unwrap();
        service.rebuild().unwrap();
        AppsProvider::new(Arc::new(service), frecency)
    }

    #[tokio::test]
    async fn queries_match_the_indexed_apps_and_rank_by_frecency() {
        let frecency = Arc::new(Frecency::in_memory().unwrap());
        let provider = provider(
            vec![entry("fake:fm", "Files"), entry("fake:ff", "Firefox"), entry("fake:c", "Calc")],
            frecency.clone(),
        );
        let titles = |items: Vec<Item>| items.into_iter().map(|i| i.title).collect::<Vec<_>>();
        assert_eq!(titles(run(&provider, "fi").await), ["Files", "Firefox"]);
        // The matched characters of the title come with the item.
        assert_eq!(run(&provider, "ffx").await[0].highlight, [0, 4, 6]);
        for _ in 0..3 {
            frecency.record("fake:ff", now()).unwrap();
        }
        assert_eq!(titles(run(&provider, "fi").await)[0], "Firefox");
        // The empty query lists what was used.
        assert_eq!(titles(run(&provider, "").await), ["Firefox"]);
    }

    fn now() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
    }

    #[test]
    fn an_item_opens_on_enter_and_carries_the_source_actions() {
        let mut app = entry("linux:fx.desktop", "Firefox");
        app.generic_name = Some("Web Browser".into());
        app.icon = Some("/icons/fx.png".into());
        app.actions = vec![AppAction {
            id: "private".into(),
            name: "Private Window".into(),
            exec: "fx -p".into(),
        }];
        let item = item_for(&app);
        assert_eq!(item.subtitle.as_deref(), Some("Web Browser"));
        assert_eq!(item.icon, Some(Icon::File { path: "/icons/fx.png".into() }));
        assert_eq!(item.actions[0].kind, ActionKind::Open);
        assert_eq!(item.actions[0].payload.as_deref(), Some("linux:fx.desktop"));
        assert_eq!(item.actions[1].payload.as_deref(), Some("linux:fx.desktop#private"));
        // Copying the command is the last action; the detail lists what Tab shows.
        let copy = item.actions.last().unwrap();
        assert_eq!((copy.kind, copy.payload.as_deref()), (ActionKind::Copy, Some("Firefox")));
        assert_eq!(
            item.detail,
            Some(Detail::Metadata {
                entries: vec![
                    ("Kind".into(), "Web Browser".into()),
                    ("Command".into(), "Firefox".into())
                ]
            })
        );
    }
}
