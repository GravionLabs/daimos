//! Running the action of a result: launching an application or one of its actions.

use std::sync::Arc;

use crate::frecency::{Frecency, FrecencyError};
use crate::index::IndexService;

/// The id of the primary action every application item has.
pub const OPEN: &str = "open";

#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("{0} is not in the index (any more)")]
    UnknownItem(String),
    #[error("cannot start it: {0}")]
    Launch(#[from] std::io::Error),
    #[error(transparent)]
    Frecency(#[from] FrecencyError),
}

pub struct ActionRunner {
    index: Arc<IndexService>,
    frecency: Arc<Frecency>,
}

impl ActionRunner {
    pub fn new(index: Arc<IndexService>, frecency: Arc<Frecency>) -> Self {
        Self { index, frecency }
    }

    /// Runs `action_id` of the item `item_id` at `now` (seconds since the epoch). Opening an
    /// application also counts as a launch for the frecency ranking; other actions do not.
    pub fn run(&self, item_id: &str, action_id: &str, now: i64) -> Result<(), ActionError> {
        let apps = self.index.apps();
        let app = apps
            .iter()
            .find(|a| a.id == item_id)
            .ok_or_else(|| ActionError::UnknownItem(item_id.to_owned()))?;
        let source = self
            .index
            .source(item_id)
            .ok_or_else(|| ActionError::UnknownItem(item_id.to_owned()))?;
        if action_id == OPEN {
            source.launch(app, None)?;
            self.frecency.record(item_id, now)?;
        } else {
            source.launch(app, Some(action_id))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::apps::{AppEntry, AppSource};
    use crate::index::AppIndex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, Option<String>)>>);

    struct Source(Arc<Recorder>);

    impl AppSource for Source {
        fn id(&self) -> &str {
            "fake"
        }
        fn scan(&self) -> Vec<AppEntry> {
            vec![AppEntry {
                id: "fake:a".into(),
                name: "A".into(),
                generic_name: None,
                keywords: vec![],
                exec: "a".into(),
                terminal: false,
                icon: None,
                actions: vec![],
            }]
        }
        fn watch_paths(&self) -> Vec<PathBuf> {
            vec![]
        }
        fn launch(&self, app: &AppEntry, action: Option<&str>) -> std::io::Result<()> {
            if action == Some("broken") {
                return Err(std::io::Error::other("boom"));
            }
            self.0.0.lock().unwrap().push((app.id.clone(), action.map(str::to_owned)));
            Ok(())
        }
    }

    fn runner() -> (ActionRunner, Arc<Recorder>, Arc<Frecency>) {
        let recorder = Arc::new(Recorder::default());
        let index = Arc::new(
            IndexService::open(
                AppIndex::in_memory().unwrap(),
                vec![Arc::new(Source(recorder.clone()))],
                None,
            )
            .unwrap(),
        );
        index.rebuild().unwrap();
        let frecency = Arc::new(Frecency::in_memory().unwrap());
        (ActionRunner::new(index, frecency.clone()), recorder, frecency)
    }

    #[test]
    fn opening_launches_the_app_and_counts_as_a_launch() {
        let (runner, recorder, frecency) = runner();
        runner.run("fake:a", OPEN, 100).unwrap();
        assert_eq!(*recorder.0.lock().unwrap(), [("fake:a".to_owned(), None)]);
        assert!(frecency.score("fake:a", 100) > 0.99);
    }

    #[test]
    fn other_actions_launch_but_do_not_count() {
        let (runner, recorder, frecency) = runner();
        runner.run("fake:a", "reveal", 100).unwrap();
        assert_eq!(*recorder.0.lock().unwrap(), [("fake:a".to_owned(), Some("reveal".to_owned()))]);
        assert_eq!(frecency.score("fake:a", 100), 0.0);
    }

    #[test]
    fn an_unknown_item_and_a_failing_launch_are_errors_and_do_not_count() {
        let (runner, _, frecency) = runner();
        assert!(matches!(runner.run("fake:gone", OPEN, 1), Err(ActionError::UnknownItem(_))));
        assert!(matches!(runner.run("nope:x", OPEN, 1), Err(ActionError::UnknownItem(_))));
        assert!(matches!(runner.run("fake:a", "broken", 1), Err(ActionError::Launch(_))));
        assert_eq!(frecency.score("fake:a", 1), 0.0);
    }
}
