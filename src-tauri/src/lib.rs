//! The Tauri shell: commands, events, tray and window lifecycle on top of `daimos-core`.

pub mod bindings;

pub fn run() {
    let builder = bindings::builder();
    tauri::Builder::default()
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Daimos");
}
