//! What the hotkey, the tray, and the CLI do to the resident window, in one place.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use daimos_platform::platform::WindowMode;
use daimos_platform::window::{TauriWindowHost, WindowHost};
use tauri::{AppHandle, Manager, Wry};
use tauri_specta::Event;

use crate::bindings::{QuerySet, WindowShown};

/// Blur events right after a show are the window system settling the focus, not the user
/// clicking away.
const BLUR_GRACE: Duration = Duration::from_millis(150);

#[derive(Default)]
pub struct ShowTimer {
    started: Mutex<Option<Instant>>,
    /// When the window was last shown; kept after `finish` so the blur grace still applies.
    shown_at: Mutex<Option<Instant>>,
}

impl ShowTimer {
    fn start(&self) {
        let now = Instant::now();
        *self.started.lock().expect("timer lock") = Some(now);
        *self.shown_at.lock().expect("timer lock") = Some(now);
    }

    fn since_start(&self) -> Option<Duration> {
        self.shown_at.lock().expect("timer lock").map(|t| t.elapsed())
    }

    /// Elapsed time of the show in flight, consumed so a second paint is not counted twice.
    pub fn finish(&self) -> Option<Duration> {
        self.started.lock().expect("timer lock").take().map(|t| t.elapsed())
    }
}

pub type Host = Arc<dyn WindowHost>;

pub fn install(app: &AppHandle<Wry>) {
    // Before the window is first mapped: a layer surface cannot be made out of a mapped window.
    let mode = app
        .get_webview_window("main")
        .map_or(WindowMode::Toplevel, |window| daimos_platform::platform::init_window(&window));
    log::info!("window mode: {mode:?}");
    app.manage(mode);
    let host: Host = Arc::new(TauriWindowHost::new(app.clone(), "main"));
    app.manage(host);
    app.manage(ShowTimer::default());
}

pub fn show(app: &AppHandle<Wry>) {
    let host = app.state::<Host>();
    app.state::<ShowTimer>().start();
    host.show();
    // The CI jobs on sway and macOS read these lines (scripts/ci/*-check.sh).
    log::info!("window visibility: shown");
    // The UI clears its query on this event; it answers with `first_paint`.
    let _ = WindowShown.emit(app);
}

/// Shows the window and puts `query` into the search field.
pub fn show_with_query(app: &AppHandle<Wry>, query: &str) {
    show(app);
    // After `show`: the UI clears its query on the show event and then takes this one.
    let _ = QuerySet(query.to_owned()).emit(app);
}

pub fn hide(app: &AppHandle<Wry>) {
    app.state::<Host>().hide();
    log::info!("window visibility: hidden");
}

pub fn toggle(app: &AppHandle<Wry>) {
    if app.state::<Host>().is_visible() {
        hide(app);
    } else {
        show(app);
    }
}

/// Hides the window when it loses the focus or is asked to close; never destroys it.
pub fn on_window_event(window: &tauri::Window<Wry>, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    let app = window.app_handle();
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(app);
        }
        tauri::WindowEvent::Focused(false) => {
            let settled = app.state::<ShowTimer>().since_start().is_none_or(|t| t > BLUR_GRACE);
            if settled {
                hide(app);
            }
        }
        tauri::WindowEvent::ThemeChanged(theme) => {
            crate::tray::follow_theme(app, *theme);
            let _ = crate::bindings::ThemeChanged(crate::bindings::theme_info(app)).emit(app);
        }
        _ => {}
    }
}
