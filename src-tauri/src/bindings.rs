//! Commands and events the webview can use. `apps/ui/src/bindings.ts` is generated from this
//! module (`pnpm bindings`) and checked in; a test fails when the file is stale.

use daimos_core::query::ResultBatch;
use daimos_core::settings::{SettingsStore, ThemeSetting};
use daimos_core::{QueryEngine, View};
use daimos_platform::hotkey::{Binding, HotkeyError, HotkeyService};
use serde::{Deserialize, Serialize};
use specta::Type;
use specta_typescript::Typescript;
use tauri::{AppHandle, Manager, State};
use tauri_specta::{Builder, Event, collect_commands, collect_events};

/// Version of the running application, shown by the UI and logged by the shell.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

#[tauri::command]
#[specta::specta]
pub fn app_info() -> AppInfo {
    AppInfo { name: "Daimos".into(), version: env!("CARGO_PKG_VERSION").into() }
}

/// Why a hotkey could not be set; `binding` is the accelerator that was refused.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HotkeyFailure {
    pub kind: String,
    pub binding: Option<String>,
    pub message: String,
}

impl From<HotkeyError> for HotkeyFailure {
    fn from(error: HotkeyError) -> Self {
        let (kind, binding) = match &error {
            HotkeyError::AlreadyTaken { binding, .. } => ("already_taken", Some(binding)),
            HotkeyError::Invalid { binding, .. } => ("invalid", Some(binding)),
            HotkeyError::Unsupported { .. } => ("unsupported", None),
            HotkeyError::Backend(_) => ("backend", None),
        };
        Self {
            kind: kind.into(),
            binding: binding.map(ToString::to_string),
            message: error.to_string(),
        }
    }
}

/// The UI hides the window (`Esc` with an empty query).
#[tauri::command]
#[specta::specta]
pub fn hide_window(app: AppHandle) {
    crate::window_control::hide(&app);
}

/// Runs an action of a result (`Enter` runs `open`). The window hides after a successful start,
/// so the application that was launched can take the focus.
#[tauri::command]
#[specta::specta]
pub fn run_action(
    app: AppHandle,
    runner: State<'_, std::sync::Arc<daimos_core::actions::ActionRunner>>,
    item_id: String,
    action_id: String,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    runner.run(&item_id, &action_id, now).map_err(|error| {
        log::warn!("action {action_id} of {item_id} failed: {error}");
        error.to_string()
    })?;
    crate::window_control::hide(&app);
    Ok(())
}

/// The view a `push` or `form` action of a result of `provider` points to.
#[tauri::command]
#[specta::specta]
pub fn load_view(
    engine: State<'_, std::sync::Arc<QueryEngine>>,
    provider: String,
    view_id: String,
) -> Result<View, String> {
    engine.view(&provider, &view_id).ok_or_else(|| format!("{provider} has no view {view_id}"))
}

/// Hands the values of a submitted form to its provider.
#[tauri::command]
#[specta::specta]
pub fn submit_form(
    engine: State<'_, std::sync::Arc<QueryEngine>>,
    provider: String,
    view_id: String,
    values: std::collections::HashMap<String, String>,
) -> Result<(), String> {
    engine.submit(&provider, &view_id, &values)
}

/// The color scheme and accent the UI should use right now.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ThemeInfo {
    /// What the user chose in the settings: `system`, `light` or `dark`.
    pub setting: ThemeSetting,
    /// The scheme of the OS, `light` or `dark`; the UI uses it while the setting is `system`.
    pub system: String,
    /// The accent color of the OS as `#rrggbb`; `None` where the OS has none (Linux).
    pub accent: Option<String>,
}

/// Emitted when the OS theme or the theme setting changes.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct ThemeChanged(pub ThemeInfo);

fn config_store(app: &AppHandle) -> Result<SettingsStore, String> {
    app.path().app_config_dir().map(SettingsStore::new).map_err(|e| e.to_string())
}

pub fn theme_info(app: &AppHandle) -> ThemeInfo {
    let setting = config_store(app)
        .ok()
        .and_then(|s| s.load().ok())
        .map_or(ThemeSetting::System, |s| s.theme);
    let system = match app.get_webview_window("main").and_then(|w| w.theme().ok()) {
        Some(tauri::Theme::Dark) => "dark",
        _ => "light",
    };
    ThemeInfo { setting, system: system.into(), accent: daimos_platform::accent::system_accent() }
}

/// The theme the UI applies at start and on every show.
#[tauri::command]
#[specta::specta]
pub fn theme(app: AppHandle) -> ThemeInfo {
    theme_info(&app)
}

/// Forces a color scheme (or `system`) and tells the UI.
#[tauri::command]
#[specta::specta]
pub fn set_theme(app: AppHandle, setting: ThemeSetting) -> Result<ThemeInfo, String> {
    let store = config_store(&app)?;
    let mut settings = store.load().unwrap_or_default();
    settings.theme = setting;
    store.save(&settings).map_err(|e| e.to_string())?;
    let info = theme_info(&app);
    let _ = ThemeChanged(info.clone()).emit(&app);
    Ok(info)
}

/// The UI reports how many result rows it shows; the window grows or shrinks to fit.
#[tauri::command]
#[specta::specta]
pub fn resize_window(host: State<'_, crate::window_control::Host>, rows: u32) {
    host.resize_for_results(rows);
}

/// The UI painted the first frame after a show: logs the time from the hotkey event to now.
#[tauri::command]
#[specta::specta]
pub fn first_paint(timer: State<'_, crate::window_control::ShowTimer>) {
    if let Some(elapsed) = timer.finish() {
        log::info!(
            "window shown: hotkey to first paint in {:.1} ms",
            elapsed.as_secs_f64() * 1000.0
        );
    }
}

/// What the UI shows about the hotkey: whether it works, which window system, and why not.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct HotkeyStatus {
    /// `active`, `pending` (waiting for the portal dialog), `unsupported`, or `failed`.
    pub state: String,
    pub session: String,
    pub binding: Option<String>,
    pub message: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub fn hotkey_status(status: State<'_, StatusCell>) -> HotkeyStatus {
    status.get()
}

/// What this system can do, so the settings page can say what is missing and why.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Capabilities {
    pub session: String,
    /// `layer_shell` or `toplevel`.
    pub window_mode: String,
    pub hotkey: HotkeyStatus,
}

#[tauri::command]
#[specta::specta]
pub fn capabilities(
    status: State<'_, StatusCell>,
    mode: State<'_, daimos_platform::platform::WindowMode>,
) -> Capabilities {
    let hotkey = status.get();
    Capabilities {
        session: hotkey.session.clone(),
        window_mode: match *mode {
            daimos_platform::platform::WindowMode::LayerShell => "layer_shell",
            daimos_platform::platform::WindowMode::Toplevel => "toplevel",
        }
        .into(),
        hotkey,
    }
}

/// Binds a new hotkey and stores it in the settings. A refused binding changes nothing.
#[tauri::command]
#[specta::specta]
pub async fn set_hotkey(
    app: AppHandle,
    hotkey: State<'_, std::sync::Arc<HotkeyService>>,
    status: State<'_, StatusCell>,
    binding: String,
) -> Result<(), HotkeyFailure> {
    let service = std::sync::Arc::clone(&hotkey);
    let wanted = Binding::new(binding.clone());
    // `bind` can wait for the portal dialog; keep it off the async runtime.
    tauri::async_runtime::spawn_blocking(move || service.bind(&wanted)).await.map_err(|e| {
        HotkeyFailure { kind: "backend".into(), binding: None, message: e.to_string() }
    })??;
    status.set(HotkeyStatus {
        state: "active".into(),
        session: status.get().session,
        binding: Some(binding.clone()),
        message: None,
    });
    let store = SettingsStore::new(app.path().app_config_dir().map_err(|e| HotkeyFailure {
        kind: "backend".into(),
        binding: None,
        message: e.to_string(),
    })?);
    let mut settings = store.load().unwrap_or_default();
    settings.hotkey = binding;
    store.save(&settings).map_err(|e| HotkeyFailure {
        kind: "backend".into(),
        binding: None,
        message: e.to_string(),
    })
}

/// The current [`HotkeyStatus`], shared between the setup thread and the commands.
pub struct StatusCell(std::sync::Mutex<HotkeyStatus>);

impl StatusCell {
    pub fn new(status: HotkeyStatus) -> Self {
        Self(std::sync::Mutex::new(status))
    }

    pub fn get(&self) -> HotkeyStatus {
        self.0.lock().expect("status lock").clone()
    }

    pub fn set(&self, status: HotkeyStatus) {
        *self.0.lock().expect("status lock") = status;
    }
}

/// Emitted when the hotkey state changes after start-up (the portal dialog was answered).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct HotkeyStatusChanged(pub HotkeyStatus);

/// The tray item `Settings` was chosen; the UI opens the settings page (M1).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct OpenSettings;

/// Whether Daimos starts hidden when the user logs in.
#[tauri::command]
#[specta::specta]
pub fn autostart_enabled(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    if enabled { launcher.enable() } else { launcher.disable() }.map_err(|e| e.to_string())
}

/// Text for the search field, sent by `daimos show "text"` right after the window was shown.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct QuerySet(pub String);

/// Emitted every time the window is shown, so the UI can clear the query without being recreated.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct WindowShown;

/// One batch of results for a query session; see `ResultBatch` for the fields.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct QueryResults(pub ResultBatch);

/// Starts a query for `session` and cancels the session's previous one. The answer streams in
/// as `QueryResults` events.
#[tauri::command]
#[specta::specta]
pub fn query(engine: State<'_, std::sync::Arc<QueryEngine>>, session: u32, text: String) {
    engine.query(session, &text);
}

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            app_info,
            hotkey_status,
            set_hotkey,
            hide_window,
            resize_window,
            first_paint,
            query,
            run_action,
            load_view,
            submit_form,
            theme,
            set_theme
        ])
        .events(collect_events![
            WindowShown,
            HotkeyStatusChanged,
            QuerySet,
            OpenSettings,
            QueryResults,
            ThemeChanged
        ])
        .typ::<View>()
}

/// Writes the TypeScript bindings to `path`.
pub fn export(path: impl AsRef<std::path::Path>) -> Result<(), Box<dyn std::error::Error>> {
    builder().export(Typescript::default(), path)?;
    Ok(())
}

/// Whether the checked-in bindings are what the Rust side generates today.
pub fn is_current(path: impl AsRef<std::path::Path>) -> Result<bool, Box<dyn std::error::Error>> {
    let generated = std::env::temp_dir().join("daimos-bindings-check.ts");
    export(&generated)?;
    let want = std::fs::read_to_string(&generated)?.replace("\r\n", "\n");
    let have = std::fs::read_to_string(path)?.replace("\r\n", "\n");
    Ok(want == have)
}
