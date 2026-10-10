//! The Tauri shell: commands, events, tray and window lifecycle on top of `daimos-core`.

pub fn run() {
    tauri::Builder::default().run(tauri::generate_context!()).expect("error while running Daimos");
}
