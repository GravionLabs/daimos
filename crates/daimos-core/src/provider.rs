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

/// The result of a successful action: what the shell does, and a message to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    pub effect: ActionEffect,
    /// A short message for the user, for example `Starred`.
    pub toast: Option<String>,
}

impl From<ActionEffect> for ActionOutcome {
    fn from(effect: ActionEffect) -> Self {
        Self { effect, toast: None }
    }
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &str;

    /// A prefix such as `gh` that routes a query to this provider only.
    fn keyword(&self) -> Option<&str> {
        None
    }

    /// Every prefix that routes a query to this provider only. Defaults to [`keyword`](Self::keyword).
    fn keywords(&self) -> Vec<&str> {
        self.keyword().into_iter().collect()
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

    /// Like [`run_action`](Self::run_action), for providers whose actions also answer with a
    /// toast. Providers that only need an effect keep the default.
    fn run_action_outcome(
        &self,
        item_id: &str,
        action_id: &str,
    ) -> Option<Result<ActionOutcome, String>> {
        self.run_action(item_id, action_id).map(|result| result.map(ActionOutcome::from))
    }

    /// Receives the values of a submitted `form` view.
    fn submit(&self, _view_id: &str, _values: &HashMap<String, String>) -> Result<(), String> {
        Err("this provider has no forms".into())
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use futures::stream;

    use super::*;

    struct Plain;

    impl Provider for Plain {
        fn id(&self) -> &str {
            "plain"
        }
        fn query(&self, _: QueryContext) -> BoxStream<'static, ItemBatch> {
            stream::empty().boxed()
        }
        fn run_action(&self, item_id: &str, _: &str) -> Option<Result<ActionEffect, String>> {
            match item_id {
                "mine" => Some(Ok(ActionEffect::Stay)),
                "broken" => Some(Err("boom".into())),
                _ => None,
            }
        }
    }

    #[test]
    fn the_default_outcome_wraps_the_effect_without_a_toast() {
        assert_eq!(
            Plain.run_action_outcome("mine", "x"),
            Some(Ok(ActionOutcome { effect: ActionEffect::Stay, toast: None }))
        );
        assert_eq!(Plain.run_action_outcome("broken", "x"), Some(Err("boom".to_owned())));
        assert_eq!(Plain.run_action_outcome("other", "x"), None);
    }
}
