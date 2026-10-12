//! The Tauri shell: commands, events, tray and window lifecycle on top of `daimos-core`.

pub mod bindings;
mod capabilities;
mod clipboard;
mod ipc_server;
mod tray;
mod ui_ready;
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
    // First, as the plugin requires. Starting Daimos again shows the running one.
    let tauri_builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| window_control::show(app)));
    // The panel manager the NSPanel conversion looks up; without it the app aborts at launch.
    #[cfg(target_os = "macos")]
    let tauri_builder = tauri_builder.plugin(tauri_nspanel::init());
    tauri_builder
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
            setup_clipboard(app)?;
            setup_query_engine(app)?;
            setup_extensions(app);
            setup_hotkey(app, started)?;
            start_index_in_background(app);
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

/// Opens the application index and the query engine on top of it. The stored list is loaded here,
/// before the hotkey is registered, so the first query already has results; the scan that
/// refreshes it runs in the background afterwards (`start_index_in_background`).
fn setup_query_engine(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use daimos_core::app_provider::AppsProvider;
    use daimos_core::frecency::Frecency;
    use daimos_core::index::{AppIndex, IndexService};
    use tauri_specta::Event;

    let data_dir = app.path().app_data_dir()?;
    let icon_dir = data_dir.join("icons");
    let index = Arc::new(IndexService::open(
        AppIndex::open(&data_dir.join("index.db"))?,
        app_sources(&icon_dir),
        Some(icon_dir),
    )?);
    log::info!("loaded {} apps from the index", index.apps().len());
    let frecency = Arc::new(Frecency::open(&data_dir.join("frecency.db"))?);

    let handle = app.handle().clone();
    let sink: daimos_core::query::Sink = Arc::new(move |batch| {
        if let Err(error) = bindings::QueryResults(batch).emit(&handle) {
            log::warn!("cannot send results: {error}");
        }
    });
    // Exchange rates: the cached ones now, a fresh copy from the ECB in the background.
    let rates =
        Arc::new(daimos_core::calc::rates::RatesCache::open(Some(data_dir.join("rates.json"))));
    refresh_rates_in_background(&rates);
    let providers: Vec<Arc<dyn daimos_core::Provider>> = vec![
        Arc::new(daimos_core::calc::provider::CalcProvider::new(rates)),
        Arc::new(AppsProvider::new(Arc::clone(&index), Arc::clone(&frecency))),
        Arc::new(daimos_core::commands::CommandsProvider::new()),
        app.state::<Arc<clipboard::ClipboardService>>().provider(),
    ];
    app.manage(Arc::new(daimos_core::actions::ActionRunner::new(Arc::clone(&index), frecency)));
    let runtime = tauri::async_runtime::handle();
    // The extension registry exists before the engine; its providers are read at every query.
    let registry = Arc::new(daimos_ext::Registry::new());
    app.manage(Arc::clone(&registry));
    let engine = Arc::new(
        daimos_core::QueryEngine::new(providers, sink, runtime.inner().clone())
            .with_dynamic(Arc::new(move || registry.providers())),
    );
    // What the user switched off or renamed in the settings applies from the first query.
    match SettingsStore::new(app.path().app_config_dir()?).load() {
        Ok(settings) => engine.set_config(daimos_core::query::EngineConfig::from(&settings)),
        Err(error) => log::warn!("settings not applied to the query engine: {error}"),
    }
    app.manage(engine);
    app.manage(index);
    Ok(())
}

/// Loads the extensions and reloads them when their files change. Script extensions in one-shot
/// mode answer queries; wasm and long-running scripts do not yet, so their commands return nothing.
fn setup_extensions(app: &tauri::App) {
    use daimos_ext::{ExtensionHost, Registry, ScriptBackend};

    let Some(root) = daimos_ext::dir::extensions_dir() else {
        log::warn!("no config directory, extensions are off");
        return;
    };
    let registry = Arc::clone(&*app.state::<Arc<Registry>>());
    // The stderr of script extensions goes to `<app data dir>/logs/<extension>.log`.
    let backend = match app.path().app_data_dir() {
        Ok(data_dir) => ScriptBackend::with_data_dir(data_dir),
        Err(error) => {
            log::warn!("no data directory, script extensions are not logged: {error}");
            ScriptBackend::new()
        }
    };
    // Preference values: text in the settings database, passwords in the OS keyring.
    let backend = match app.path().app_config_dir() {
        Ok(config_dir) => {
            let preferences = Arc::new(daimos_ext::Preferences::new(
                SettingsStore::new(config_dir),
                Arc::new(daimos_ext::OsSecretStore),
            ));
            app.manage(Arc::clone(&preferences));
            backend.with_preferences(preferences)
        }
        Err(error) => {
            log::warn!("no config directory, extension preferences are off: {error}");
            backend
        }
    };
    let host = Arc::new(ExtensionHost::new(root, Arc::new(backend), registry));
    match host.watch(std::time::Duration::from_millis(250)) {
        Ok(watcher) => {
            app.manage(watcher);
        }
        Err(error) => log::warn!("cannot watch the extension directory: {error}"),
    }
}

/// Opens the clipboard history and starts following the clipboard when the setting is on.
fn setup_clipboard(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let settings = SettingsStore::new(app.path().app_config_dir()?).load().unwrap_or_default();
    let history = Arc::new(daimos_core::clipboard::ClipboardHistory::open(
        &app.path().app_data_dir()?.join("clipboard.db"),
        clipboard::limits(&settings.clipboard),
    )?);
    let service = clipboard::ClipboardService::new(
        daimos_platform::clipboard::backend(Session::detect()),
        history,
    );
    service.apply(&settings.clipboard);
    app.manage(Arc::new(service));
    let paster: Arc<dyn daimos_core::paste::Paster> =
        daimos_platform::paste::paster(Session::detect());
    app.manage(paster);
    Ok(())
}

/// Fetches the daily exchange rates when the cached ones are missing or old. Offline is fine: the
/// calculator then uses what it has, and the item says how old that is.
fn refresh_rates_in_background(rates: &Arc<daimos_core::calc::rates::RatesCache>) {
    let rates = Arc::clone(rates);
    let _ = std::thread::Builder::new().name("daimos-rates".into()).spawn(move || {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        match rates.refresh(now, daimos_core::calc::rates::fetch_ecb) {
            Ok(()) => {
                log::info!("exchange rates of {}", rates.get().map_or("none".into(), |r| r.date))
            }
            Err(error) => log::info!("no fresh exchange rates ({error}); using the cached ones"),
        }
    });
}

/// The application sources of this OS.
fn app_sources(icon_dir: &std::path::Path) -> Vec<Arc<dyn daimos_core::apps::AppSource>> {
    #[cfg(target_os = "linux")]
    {
        let _ = icon_dir;
        vec![Arc::new(daimos_platform::apps::linux::LinuxAppSource::new())]
    }
    #[cfg(windows)]
    {
        vec![Arc::new(daimos_platform::apps::windows::WindowsAppSource::new(icon_dir))]
    }
    #[cfg(target_os = "macos")]
    {
        vec![Arc::new(daimos_platform::apps::macos::MacAppSource::new(icon_dir))]
    }
}

/// Scans in the background and again whenever an application directory changes.
fn start_index_in_background(app: &tauri::App) {
    if let Some(index) = app.try_state::<Arc<daimos_core::index::IndexService>>() {
        index.start_background(daimos_core::index::DEBOUNCE);
    }
}

/// Quit from the tray or the CLI: stop the socket listener and give the hotkey back.
fn shutdown(app: &tauri::AppHandle) {
    ipc_server::stop(app);
    if let Some(clipboard) = app.try_state::<Arc<clipboard::ClipboardService>>() {
        clipboard.stop();
    }
    if let Some(index) = app.try_state::<Arc<daimos_core::index::IndexService>>() {
        index.stop();
    }
    if let Some(watcher) = app.try_state::<daimos_ext::ExtensionWatcher>() {
        watcher.stop();
    }
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
                capabilities::bound_status(
                    &session_name,
                    &binding,
                    service.backend().reported_trigger(),
                )
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
