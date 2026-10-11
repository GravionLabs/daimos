//! The built-in commands of the palette itself, found by typing their name.

use futures::StreamExt;
use futures::stream::{self, BoxStream};

use crate::frecency::Frecency;
use crate::model::{Action, ActionKind, Icon, Item, ItemBatch};
use crate::provider::{Provider, QueryContext};
use crate::ranker::{Candidate, Ranker};

/// The payload of the action that opens the settings page; the UI handles it itself.
pub const OPEN_SETTINGS: &str = "daimos:settings";

pub struct CommandsProvider {
    ranker: std::sync::Mutex<Ranker>,
    /// Commands are not ranked by use, so this stays empty.
    frecency: Frecency,
}

impl CommandsProvider {
    pub fn new() -> Self {
        Self {
            ranker: std::sync::Mutex::new(Ranker::new()),
            frecency: Frecency::in_memory().expect("in-memory frecency"),
        }
    }

    fn items() -> Vec<Item> {
        let mut settings = Item::new("cmd:settings", "Settings");
        settings.subtitle = Some("Hotkey, theme, providers, aliases".into());
        settings.keywords =
            ["preferences", "options", "configuration", "hotkey", "theme", "aliases"]
                .map(str::to_owned)
                .to_vec();
        settings.icon = Some(Icon::Named { name: "⚙".into() });
        settings.actions = vec![Action {
            id: "open".into(),
            title: "Open Settings".into(),
            shortcut: Some("Enter".into()),
            kind: ActionKind::Push,
            payload: Some(OPEN_SETTINGS.into()),
        }];
        vec![settings]
    }
}

impl Default for CommandsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for CommandsProvider {
    fn id(&self) -> &str {
        "commands"
    }

    /// Below the apps: typing `set` should find Settings, but not before an app of that name.
    fn weight(&self) -> f32 {
        0.5
    }

    fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
        let items = Self::items();
        // Commands only appear for a query: the empty query lists the frecent apps.
        let ranked = if ctx.text.trim().is_empty() {
            Vec::new()
        } else {
            let candidates: Vec<_> =
                items.iter().map(|item| Candidate { item, weight: 1.0 }).collect();
            self.ranker.lock().unwrap_or_else(|e| e.into_inner()).rank(
                &ctx.text,
                &candidates,
                &self.frecency,
                0,
                5,
            )
        };
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

    async fn run(text: &str) -> Vec<Item> {
        let ctx = QueryContext {
            text: text.into(),
            keyword_routed: false,
            cancel: CancellationToken::new(),
        };
        CommandsProvider::new()
            .query(ctx)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .flat_map(|b| b.items)
            .collect()
    }

    #[tokio::test]
    async fn settings_is_found_by_name_and_by_what_it_contains() {
        for text in ["set", "settings", "pref", "theme", "hotkey"] {
            let items = run(text).await;
            assert_eq!(items.len(), 1, "{text}");
            assert_eq!(items[0].id, "cmd:settings");
        }
        assert_eq!(run("set").await[0].highlight, [0, 1, 2]);
    }

    #[tokio::test]
    async fn the_empty_query_and_unrelated_text_find_nothing() {
        for text in ["", "  ", "firefox", "zzz"] {
            assert!(run(text).await.is_empty(), "{text:?}");
        }
    }

    #[tokio::test]
    async fn the_item_opens_the_settings_page() {
        let item = run("settings").await.remove(0);
        let action = &item.actions[0];
        assert_eq!(
            (action.kind, action.payload.as_deref()),
            (ActionKind::Push, Some(OPEN_SETTINGS))
        );
    }
}
