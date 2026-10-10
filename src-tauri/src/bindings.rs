//! Commands and events the webview can use. `apps/ui/src/bindings.ts` is generated from this
//! module (`pnpm bindings`) and checked in; a test fails when the file is stale.

use daimos_core::settings::SettingsStore;
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

/// Binds a new hotkey and stores it in the settings. A refused binding changes nothing.
#[tauri::command]
#[specta::specta]
pub fn set_hotkey(
    app: AppHandle,
    hotkey: State<'_, HotkeyService>,
    binding: String,
) -> Result<(), HotkeyFailure> {
    hotkey.bind(&Binding::new(binding.clone()))?;
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

/// Emitted every time the window is shown, so the UI can clear the query without being recreated.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct WindowShown;

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![app_info, set_hotkey])
        .events(collect_events![WindowShown])
}

/// Writes the TypeScript bindings to `path`.
pub fn export(path: impl AsRef<std::path::Path>) -> Result<(), Box<dyn std::error::Error>> {
    builder().export(Typescript::default(), path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checked-in bindings must be what the Rust side generates today.
    #[test]
    fn checked_in_bindings_are_current() {
        let checked_in =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../apps/ui/src/bindings.ts");
        let generated = std::env::temp_dir().join("daimos-bindings-check.ts");
        export(&generated).expect("export bindings");
        let want = std::fs::read_to_string(&generated).expect("generated file");
        let have = std::fs::read_to_string(&checked_in).expect("apps/ui/src/bindings.ts exists");
        assert_eq!(
            have.replace("\r\n", "\n"),
            want.replace("\r\n", "\n"),
            "bindings.ts is stale: run `pnpm bindings`"
        );
    }
}
