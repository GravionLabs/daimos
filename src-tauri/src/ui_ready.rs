//! Events sent before the web view listens are lost. This holds back `window-shown` and
//! `query-set` until the UI says it listens (`ui_ready`), then replays what it missed: a
//! `daimos show "text"` right after the app started still fills the search field.

use std::sync::Mutex;

#[derive(Default)]
struct State {
    ready: bool,
    shown: bool,
    query: Option<String>,
}

#[derive(Default)]
pub struct UiReadiness {
    state: Mutex<State>,
}

impl UiReadiness {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The window was shown. `true`: the UI listens, emit the event now. `false`: it is kept for
    /// [`UiReadiness::ready`].
    pub fn shown(&self) -> bool {
        let mut state = self.lock();
        if state.ready {
            return true;
        }
        state.shown = true;
        // A new show starts from an empty search field: an earlier text no longer applies.
        state.query = None;
        false
    }

    /// Text for the search field, with the same contract as [`UiReadiness::shown`].
    pub fn query(&self, text: &str) -> bool {
        let mut state = self.lock();
        if state.ready {
            return true;
        }
        state.query = Some(text.to_owned());
        false
    }

    /// The UI listens now. Returns what it missed: whether the window was shown, and the text.
    pub fn ready(&self) -> (bool, Option<String>) {
        let mut state = self.lock();
        state.ready = true;
        (std::mem::take(&mut state.shown), state.query.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn before_the_ui_listens_events_are_kept_and_replayed_once() {
        let ui = UiReadiness::default();
        assert!(!ui.shown());
        assert!(!ui.query("firefox"));
        assert_eq!(ui.ready(), (true, Some("firefox".to_owned())));
        // Nothing is replayed twice.
        assert_eq!(ui.ready(), (false, None));
    }

    #[test]
    fn after_the_ui_listens_events_go_straight_through() {
        let ui = UiReadiness::default();
        ui.ready();
        assert!(ui.shown());
        assert!(ui.query("x"));
        assert_eq!(ui.ready(), (false, None));
    }

    #[test]
    fn only_the_last_text_of_the_last_show_is_replayed() {
        let ui = UiReadiness::default();
        ui.shown();
        ui.query("first");
        ui.query("second");
        assert_eq!(ui.ready(), (true, Some("second".to_owned())));

        // A show without text after a show with text: the old text is stale.
        let ui = UiReadiness::default();
        ui.shown();
        ui.query("old");
        ui.shown();
        assert_eq!(ui.ready(), (true, None));
    }

    #[test]
    fn a_text_without_a_show_is_still_replayed() {
        let ui = UiReadiness::default();
        ui.query("lonely");
        assert_eq!(ui.ready(), (false, Some("lonely".to_owned())));
    }
}
