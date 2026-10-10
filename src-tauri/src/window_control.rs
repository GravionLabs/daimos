//! What the hotkey, the tray, and the CLI do to the resident window, in one place.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
}

impl ShowTimer {
    fn start(&self) {
        *self.started.lock().expect("timer lock") = Some(Instant::now());
    }

    fn since_start(&self) -> Option<Duration> {
        self.started.lock().expect("timer lock").map(|t| t.elapsed())
    }

    /// Elapsed time of the show in flight, consumed so a second paint is not counted twice.
    pub fn finish(&self) -> Option<Duration> {
        self.started.lock().expect("timer lock").take().map(|t| t.elapsed())
    }
}

pub type Host = Arc<dyn WindowHost>;

pub fn install(app: &AppHandle<Wry>) {
    let host: Host = Arc::new(TauriWindowHost::new(app.clone(), "main"));
    app.manage(host);
    app.manage(ShowTimer::default());
}

pub fn show(app: &AppHandle<Wry>) {
    let host = app.state::<Host>();
    app.state::<ShowTimer>().start();
    host.show();
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
        _ => {}
    }
}
