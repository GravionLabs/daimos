//! The calculator as a provider: at most one item, the result, copied with `Enter`.

use std::sync::Arc;

use futures::StreamExt;
use futures::stream::{self, BoxStream};

use super::evaluate;
use super::rates::RatesCache;
use crate::model::{Action, ActionKind, Icon, Item, ItemBatch};
use crate::provider::{Provider, QueryContext};

pub struct CalcProvider {
    rates: Arc<RatesCache>,
}

impl CalcProvider {
    pub fn new(rates: Arc<RatesCache>) -> Self {
        Self { rates }
    }
}

impl Provider for CalcProvider {
    fn id(&self) -> &str {
        "calc"
    }

    /// A result is what the user is after when they typed a calculation: it goes above the apps.
    fn weight(&self) -> f32 {
        10.0
    }

    fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch> {
        let rates = self.rates.get();
        let items: Vec<Item> = evaluate(&ctx.text, rates.as_ref())
            .map(|answer| {
                let mut item = Item::new("calc:result", answer.title);
                item.subtitle = Some(answer.subtitle);
                // A single symbol is drawn as the icon itself.
                item.icon = Some(Icon::Named { name: "=".into() });
                item.actions = vec![Action {
                    id: "copy".into(),
                    title: "Copy Result".into(),
                    shortcut: Some("Enter".into()),
                    kind: ActionKind::Copy,
                    payload: Some(answer.copy),
                }];
                item
            })
            .into_iter()
            .collect();
        stream::once(async move { ItemBatch::from(items) }).boxed()
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::*;

    async fn run(text: &str) -> Vec<Item> {
        let provider = CalcProvider::new(Arc::new(RatesCache::open(None)));
        let ctx = QueryContext {
            text: text.into(),
            keyword_routed: false,
            cancel: CancellationToken::new(),
        };
        provider.query(ctx).collect::<Vec<_>>().await.into_iter().flat_map(|b| b.items).collect()
    }

    #[tokio::test]
    async fn an_expression_gives_one_item_whose_primary_action_copies_the_result() {
        let items = run("2*(3+4)").await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "14");
        assert_eq!(items[0].subtitle.as_deref(), Some("2*(3+4)"));
        assert_eq!(items[0].icon, Some(Icon::Named { name: "=".into() }));
        let action = &items[0].actions[0];
        assert_eq!((action.kind, action.payload.as_deref()), (ActionKind::Copy, Some("14")));
    }

    #[tokio::test]
    async fn anything_else_gives_no_item() {
        for text in ["", "firefox", "42", "calc", "2 +"] {
            assert!(run(text).await.is_empty(), "{text:?}");
        }
    }

    #[test]
    fn the_calculator_outweighs_the_apps() {
        let provider = CalcProvider::new(Arc::new(RatesCache::open(None)));
        assert!(provider.weight() > 1.0);
    }
}
