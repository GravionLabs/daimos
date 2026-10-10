//! The Tauri shell: commands, events, tray and window lifecycle on top of `daimos-core`.

pub mod bindings;

use daimos_core::settings::SettingsStore;
use daimos_platform::hotkey::{self, Binding, HotkeyService};
use tauri::Manager;

pub fn run() {
    let builder = bindings::builder();
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            setup_hotkey(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Daimos");
}

/// Registers the stored hotkey. A refused or unsupported binding is logged and shown by the UI;
/// it must not stop the app, because the CLI and the tray still work.
fn setup_hotkey(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let store = SettingsStore::new(app.path().app_config_dir()?);
    let settings = store.load().unwrap_or_else(|error| {
        log::warn!("settings unreadable, using defaults: {error}");
        Default::default()
    });

    let service = HotkeyService::new(hotkey::detect(app.handle()));
    let presses = service.backend().presses();
    match service.bind(&Binding::new(settings.hotkey)) {
        Ok(()) => log::info!("hotkey registered"),
        Err(error) => log::warn!("hotkey not registered: {error}"),
    }
    app.manage(service);

    // The window toggle arrives with the resident-window work; until then a press is only logged.
    std::thread::spawn(move || {
        for press in presses {
            log::info!("hotkey pressed: {}", press.binding);
        }
    });
    Ok(())
}
