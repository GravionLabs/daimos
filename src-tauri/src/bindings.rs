//! Commands and events the webview can use. `apps/ui/src/bindings.ts` is generated from this
//! module (`pnpm bindings`) and checked in; a test fails when the file is stale.

use serde::{Deserialize, Serialize};
use specta::Type;
use specta_typescript::Typescript;
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

/// Emitted every time the window is shown, so the UI can clear the query without being recreated.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct WindowShown;

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![app_info])
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
