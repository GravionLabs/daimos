//! The Tauri shell: commands, events, tray and window lifecycle on top of `daimos-core`.

pub mod bindings;
mod ipc_server;
mod tray;
mod window_control;

use daimos_core::settings::SettingsStore;
use std::sync::Arc;

use bindings::{HotkeyStatus, HotkeyStatusChanged, StatusCell};
use daimos_platform::hotkey::{self, Binding, HotkeyError, HotkeyService};
use daimos_platform::session::Session;
use tauri::Manager;
use tauri_specta::Event;

pub fn run() {
    // Process start to hotkey ready is a budget of the product spec (500 ms).
    let started = std::time::Instant::now();
    let builder = bindings::builder();
    tauri::Builder::default()
        // First, as the plugin requires. Starting Daimos again shows the running one.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| window_control::show(app)))
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .invoke_handler(builder.invoke_handler())
        .on_window_event(window_control::on_window_event)
        .setup(move |app| {
            builder.mount_events(app);
            window_control::install(app.handle());
            // No tray (a desktop without a status notifier host) must not stop the hotkey and the CLI.
            if let Err(error) = tray::install(app.handle()) {
                log::warn!("no tray icon: {error}");
            }
            setup_hotkey(app, started)?;
            ipc_server::start(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Daimos")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                shutdown(app);
            }
        });
}

/// Quit from the tray or the CLI: stop the socket listener and give the hotkey back.
fn shutdown(app: &tauri::AppHandle) {
    ipc_server::stop(app);
    if let Some(hotkey) = app.try_state::<Arc<HotkeyService>>() {
        let _ = hotkey.release();
    }
}

/// Registers the stored hotkey on a background thread: on Wayland the first registration waits
/// for the portal dialog. A refused or unsupported binding is logged and shown by the UI; it must
/// not stop the app, because the CLI and the tray still work.
fn setup_hotkey(
    app: &tauri::App,
    started: std::time::Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = SettingsStore::new(app.path().app_config_dir()?);
    let settings = store.load().unwrap_or_else(|error| {
        log::warn!("settings unreadable, using defaults: {error}");
        Default::default()
    });

    let session = Session::detect();
    let session_name = format!("{session:?}").to_lowercase();
    let service = Arc::new(HotkeyService::new(hotkey::detect(app.handle())));
    let presses = service.backend().presses();
    app.manage(StatusCell::new(HotkeyStatus {
        state: "pending".into(),
        session: session_name.clone(),
        binding: Some(settings.hotkey.clone()),
        message: None,
    }));
    app.manage(Arc::clone(&service));

    let handle = app.handle().clone();
    std::thread::spawn(move || {
        let binding = Binding::new(settings.hotkey);
        let status = match service.bind(&binding) {
            Ok(()) => {
                log::info!(
                    "hotkey {binding} registered {} ms after process start",
                    started.elapsed().as_millis()
                );
                HotkeyStatus {
                    state: "active".into(),
                    session: session_name,
                    binding: Some(binding.to_string()),
                    message: None,
                }
            }
            Err(error) => {
                log::warn!("hotkey not registered: {error}");
                let state = match error {
                    HotkeyError::Unsupported { .. } => "unsupported",
                    _ => "failed",
                };
                HotkeyStatus {
                    state: state.into(),
                    session: session_name,
                    binding: Some(binding.to_string()),
                    message: Some(error.to_string()),
                }
            }
        };
        handle.state::<StatusCell>().set(status.clone());
        let _ = HotkeyStatusChanged(status).emit(&handle);
    });

    let handle = app.handle().clone();
    std::thread::spawn(move || {
        for press in presses {
            log::debug!("hotkey pressed: {}", press.binding);
            window_control::toggle(&handle);
        }
    });
    Ok(())
}
