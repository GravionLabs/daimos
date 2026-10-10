//! Per-OS hooks of the window host. Each platform PBI replaces its branch; the rest do nothing.

use serde::Serialize;
use tauri::{Runtime, WebviewWindow};

use crate::window::PreviousWindow;

/// How the window is presented. Reported to the UI as a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WindowMode {
    /// A `wlr-layer-shell` surface on the overlay layer (wlroots compositors, KDE Plasma).
    LayerShell,
    /// A normal toplevel window (X11, Windows, macOS, and GNOME on Wayland).
    Toplevel,
}

/// Prepares the window once, right after it was created and before it is first mapped.
pub fn init_window<R: Runtime>(window: &WebviewWindow<R>) -> WindowMode {
    #[cfg(target_os = "macos")]
    if let Err(error) = crate::macos::convert(window) {
        log::warn!("cannot turn the window into a panel: {error}");
    }
    #[cfg(all(target_os = "linux", feature = "layer-shell"))]
    return layer_shell::init(window);
    #[cfg(not(all(target_os = "linux", feature = "layer-shell")))]
    {
        let _ = window;
        WindowMode::Toplevel
    }
}

/// The window that is active right now, called before the Daimos window is shown.
pub(crate) fn previous_window() -> Option<PreviousWindow> {
    #[cfg(windows)]
    return crate::windows::foreground();
    #[cfg(not(windows))]
    None
}

/// Called right after the window was shown (activation policy, foreground, window class).
/// Returns `true` when the platform already gave the window the keyboard focus, so the caller
/// must not call `set_focus` (on macOS that activates the whole application).
pub(crate) fn after_show<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    #[cfg(target_os = "macos")]
    return crate::macos::show_and_make_key(window.app_handle(), window.label());
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        // The raw handle type of Tauri's `windows` dependency and ours are the same struct.
        let hwnd = windows::Win32::Foundation::HWND(hwnd.0);
        crate::windows::apply_palette_style(hwnd);
        if !crate::windows::take_foreground(hwnd) {
            log::warn!("Windows did not give the foreground to the window");
        }
    }
    #[cfg(windows)]
    return true;
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = window;
        false
    }
}

#[cfg(all(target_os = "linux", feature = "layer-shell"))]
mod layer_shell {
    use gtk_layer_shell::{KeyboardMode, Layer, LayerShell};

    use super::*;

    /// Turns the GTK window into an overlay layer surface when the compositor offers
    /// `wlr-layer-shell`. No anchor means the compositor centers it on the output; exclusive
    /// keyboard interactivity gives the search field the keys while it is shown.
    pub(super) fn init<R: Runtime>(window: &WebviewWindow<R>) -> WindowMode {
        if !gtk_layer_shell::is_supported() {
            log::info!("no wlr-layer-shell here, the window stays a toplevel");
            return WindowMode::Toplevel;
        }
        let Ok(gtk_window) = window.gtk_window() else {
            return WindowMode::Toplevel;
        };
        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Overlay);
        gtk_window.set_keyboard_mode(KeyboardMode::Exclusive);
        gtk_window.set_namespace("daimos");
        log::info!("the window is a wlr-layer-shell overlay surface");
        WindowMode::LayerShell
    }
}
