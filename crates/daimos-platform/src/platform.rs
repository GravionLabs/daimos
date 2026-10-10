//! Per-OS hooks of the window host. Each platform PBI replaces its branch; the rest do nothing.

use tauri::{Runtime, WebviewWindow};

use crate::window::PreviousWindow;

/// The window that is active right now, called before the Daimos window is shown.
pub(crate) fn previous_window() -> Option<PreviousWindow> {
    None
}

/// Called right after the window was shown (activation policy, foreground, window class).
pub(crate) fn after_show<R: Runtime>(_window: &WebviewWindow<R>) {}
