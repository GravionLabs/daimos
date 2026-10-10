//! The resident window: created once, hidden at start, and only shown and hidden afterwards.
//!
//! [`WindowHost`] is what the hotkey, the tray, and the CLI call. Each platform fills
//! [`PreviousWindow`] before the window is shown, so that actions such as paste can act on the
//! window that was active before Daimos opened.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

/// Width of the window in logical pixels.
pub const WIDTH: f64 = 720.0;
const SEARCH_ROW_HEIGHT: f64 = 76.0;
const RESULT_ROW_HEIGHT: f64 = 52.0;
/// The list never grows past this many rows; the rest scrolls.
const MAX_VISIBLE_RESULTS: u32 = 8;

/// Height in logical pixels for a result list of `count` rows.
pub fn height_for(count: u32) -> f64 {
    SEARCH_ROW_HEIGHT + f64::from(count.min(MAX_VISIBLE_RESULTS)) * RESULT_ROW_HEIGHT
}

/// Top-left corner that centers a window of `size` on a monitor, in physical pixels.
pub fn centered(
    monitor_position: (i32, i32),
    monitor_size: (u32, u32),
    size: (u32, u32),
) -> (i32, i32) {
    let offset = |monitor: u32, window: u32| (i64::from(monitor) - i64::from(window)) / 2;
    (
        monitor_position.0 + offset(monitor_size.0, size.0) as i32,
        monitor_position.1 + offset(monitor_size.1, size.1) as i32,
    )
}

/// The window that had the focus before Daimos opened. The value is platform-specific and only
/// the platform module that produced it knows how to use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum PreviousWindow {
    /// A Win32 `HWND`.
    Windows(isize),
    /// The process id of the frontmost application.
    MacOs(i32),
    /// An X11 window id.
    X11(u64),
}

pub trait WindowHost: Send + Sync {
    fn show(&self);
    fn hide(&self);
    fn is_visible(&self) -> bool;
    fn toggle(&self) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }
    /// Moves the window to the center of the monitor that holds the mouse cursor.
    fn move_to_cursor_monitor(&self);
    /// Resizes the window to fit `count` result rows.
    fn resize_for_results(&self, count: u32);
    /// The window that was active before the last `show`, if the platform could find out.
    fn previous_window(&self) -> Option<PreviousWindow>;
}

/// [`WindowHost`] over the Tauri window with the label `main`.
pub struct TauriWindowHost<R: Runtime> {
    app: AppHandle<R>,
    label: String,
    previous: Arc<Mutex<Option<PreviousWindow>>>,
}

impl<R: Runtime> TauriWindowHost<R> {
    pub fn new(app: AppHandle<R>, label: impl Into<String>) -> Self {
        Self { app, label: label.into(), previous: Arc::default() }
    }

    /// The body of `show`; runs on the main thread.
    fn show_here(&self) {
        let Some(window) = self.window() else { return };
        // Before the window exists on screen, so the platform still sees the old foreground.
        *self.previous.lock().expect("previous window lock") = crate::platform::previous_window();
        self.move_to_cursor_monitor();
        if let Err(error) = window.show() {
            log::warn!("cannot show the window: {error}");
        }
        let focused = crate::platform::after_show(&window);
        if !focused && let Err(error) = window.set_focus() {
            log::warn!("cannot focus the window: {error}");
        }
    }

    fn window(&self) -> Option<WebviewWindow<R>> {
        self.app.get_webview_window(&self.label)
    }
}

impl<R: Runtime> WindowHost for TauriWindowHost<R> {
    fn show(&self) {
        // AppKit and Win32 focus calls belong on the thread that owns the window, and `show` is
        // called from the hotkey, socket, and tray threads.
        let host = Self {
            app: self.app.clone(),
            label: self.label.clone(),
            previous: Arc::clone(&self.previous),
        };
        let _ = self.app.run_on_main_thread(move || host.show_here());
    }

    fn hide(&self) {
        if let Some(Err(error)) = self.window().map(|w| w.hide()) {
            log::warn!("cannot hide the window: {error}");
        }
    }

    fn is_visible(&self) -> bool {
        self.window().and_then(|w| w.is_visible().ok()).unwrap_or(false)
    }

    fn move_to_cursor_monitor(&self) {
        let Some(window) = self.window() else { return };
        let monitor = self
            .app
            .cursor_position()
            .ok()
            .and_then(|cursor| self.app.monitor_from_point(cursor.x, cursor.y).ok().flatten())
            .or_else(|| window.primary_monitor().ok().flatten());
        let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else { return };
        let (x, y) = centered(
            (monitor.position().x, monitor.position().y),
            (monitor.size().width, monitor.size().height),
            (size.width, size.height),
        );
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }

    fn resize_for_results(&self, count: u32) {
        let Some(window) = self.window() else { return };
        let scale = window.scale_factor().unwrap_or(1.0);
        let size = PhysicalSize::new(
            (WIDTH * scale).round() as u32,
            (height_for(count) * scale).round() as u32,
        );
        let _ = window.set_size(size);
        if window.is_visible().unwrap_or(false) {
            self.move_to_cursor_monitor();
        }
    }

    fn previous_window(&self) -> Option<PreviousWindow> {
        self.previous.lock().expect("previous window lock").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_grows_with_the_result_count_up_to_the_cap() {
        assert_eq!(height_for(0), 76.0);
        assert_eq!(height_for(3), 76.0 + 3.0 * 52.0);
        assert_eq!(height_for(8), height_for(500));
    }

    #[test]
    fn centers_on_a_monitor_at_the_origin() {
        assert_eq!(centered((0, 0), (1920, 1080), (720, 128)), (600, 476));
    }

    #[test]
    fn centers_on_a_monitor_left_of_the_origin() {
        assert_eq!(centered((-1920, 0), (1920, 1080), (720, 128)), (-1320, 476));
    }

    #[test]
    fn a_window_larger_than_the_monitor_does_not_overflow() {
        assert_eq!(centered((0, 0), (500, 100), (720, 128)), (-110, -14));
    }
}
