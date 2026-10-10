//! macOS: the window becomes a non-activating `NSPanel`.
//!
//! Showing it must not make Daimos the active application, otherwise the previous app loses its
//! menu bar and "act on the current window" (paste, later) has no target. The panel joins every
//! Space, shows over full-screen apps, floats, and hides when it stops being the key window.

use tauri::{AppHandle, Runtime, WebviewWindow};
use tauri_nspanel::{
    CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt, tauri_panel,
};

tauri_panel! {
    panel!(DaimosPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false,
            is_floating_panel: true
        }
    })
}

/// Converts the window to a panel. Call once, right after the window was created.
pub fn convert<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    let panel = window.to_panel::<DaimosPanel<R>>().map_err(|e| e.to_string())?;
    panel.set_level(PanelLevel::Floating.into());
    panel
        .add_style_mask(StyleMask::empty().nonactivating_panel().into())
        .map_err(|e| format!("{e:?}"))?;
    panel.set_collection_behavior(
        CollectionBehavior::new().can_join_all_spaces().full_screen_auxiliary().into(),
    );
    Ok(())
}

/// Orders the panel in and makes it the key window without activating the application.
pub fn show_and_make_key<R: Runtime>(app: &AppHandle<R>, label: &str) -> bool {
    match app.get_webview_panel(label) {
        Ok(panel) => {
            panel.show_and_make_key();
            true
        }
        Err(_) => false,
    }
}

/// Pasting into the previous app posts key events with `CGEvent`, which needs the Accessibility
/// permission. That arrives with the `Paster` trait (#61); nothing calls this before then.
#[allow(dead_code)]
pub fn paste_into_previous_app() -> ! {
    todo!("needs the Accessibility permission, see #61")
}
