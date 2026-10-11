//! The provider contract: built-in providers and extension hosts implement it.

use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use std::collections::HashMap;

use crate::model::{ItemBatch, View};

/// What a provider receives for one query.
#[derive(Debug, Clone)]
pub struct QueryContext {
    /// The text without the provider keyword.
    pub text: String,
    /// Set when the provider was addressed by its keyword.
    pub keyword_routed: bool,
    /// Cancelled when a newer query replaces this one or the budget runs out.
    pub cancel: CancellationToken,
}

/// What the shell does after a provider ran an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionEffect {
    /// The window hides (the action did its job, for example a paste).
    Hide,
    /// The window stays and the list is refreshed (the action changed the list, for example pin).
    Stay,
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &str;

    /// A prefix such as `gh` that routes a query to this provider only.
    fn keyword(&self) -> Option<&str> {
        None
    }

    /// Blends into the ranking (1.0 is neutral).
    fn weight(&self) -> f32 {
        1.0
    }

    /// Time the provider gets per query. Partial results are kept when it runs out.
    fn budget(&self) -> std::time::Duration {
        crate::query::DEFAULT_BUDGET
    }

    fn query(&self, ctx: QueryContext) -> BoxStream<'static, ItemBatch>;

    /// The view a `push` or `form` action points to. Providers without views keep the default.
    fn view(&self, _view_id: &str) -> Option<View> {
        None
    }

    /// Runs an action of one of its items. `None` means the item is not this provider's.
    fn run_action(&self, _item_id: &str, _action_id: &str) -> Option<Result<ActionEffect, String>> {
        None
    }

    /// Receives the values of a submitted `form` view.
    fn submit(&self, _view_id: &str, _values: &HashMap<String, String>) -> Result<(), String> {
        Err("this provider has no forms".into())
    }
}
