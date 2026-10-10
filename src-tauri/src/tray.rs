//! Tray icon with `Show Daimos`, `Settings`, and `Quit`; the icon follows the OS theme.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Theme, Wry};
use tauri_specta::Event;

use crate::bindings::OpenSettings;
use crate::window_control;

const ID_SHOW: &str = "show";
const ID_SETTINGS: &str = "settings";
const ID_QUIT: &str = "quit";
const TRAY_ID: &str = "main";

/// Light glyph for dark panels, dark glyph for light panels. macOS draws the dark one as a
/// template image, so the menu bar tints it itself.
const ICON_FOR_DARK_PANEL: &[u8] = include_bytes!("../icons/tray-light.png");
const ICON_FOR_LIGHT_PANEL: &[u8] = include_bytes!("../icons/tray-dark.png");

pub fn icon_bytes(theme: Theme) -> &'static [u8] {
    match theme {
        Theme::Dark => ICON_FOR_DARK_PANEL,
        _ => ICON_FOR_LIGHT_PANEL,
    }
}

pub fn install(app: &AppHandle<Wry>) -> tauri::Result<TrayIcon<Wry>> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, ID_SHOW, "Show Daimos", true, None::<&str>)?,
            &MenuItem::with_id(app, ID_SETTINGS, "Settings", true, None::<&str>)?,
            &MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?,
        ],
    )?;
    let theme = app.get_webview_window("main").and_then(|w| w.theme().ok()).unwrap_or(Theme::Light);
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(icon_bytes(theme))?)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Daimos")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            ID_SHOW => window_control::show(app),
            ID_SETTINGS => {
                window_control::show(app);
                let _ = OpenSettings.emit(app);
            }
            // The exit stops the socket listener and gives the hotkey back (see `shutdown`).
            ID_QUIT => app.exit(0),
            _ => {}
        })
        .build(app)
}

/// Swaps the icon when the OS theme changes.
pub fn follow_theme(app: &AppHandle<Wry>, theme: Theme) {
    if let Some(tray) = app.tray_by_id(TRAY_ID)
        && let Ok(icon) = Image::from_bytes(icon_bytes(theme))
    {
        let _ = tray.set_icon(Some(icon));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_panels_get_the_light_glyph_and_vice_versa() {
        assert_eq!(icon_bytes(Theme::Dark), ICON_FOR_DARK_PANEL);
        assert_eq!(icon_bytes(Theme::Light), ICON_FOR_LIGHT_PANEL);
        assert_ne!(ICON_FOR_DARK_PANEL, ICON_FOR_LIGHT_PANEL);
    }
}
