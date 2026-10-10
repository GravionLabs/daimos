//! Registers a real binding through the Tauri plugin, asserts it, and unregisters it.
//! Needs a display on Linux (Xvfb in CI), so it skips without one.
//!
//! One test, one app: the OS allows a single hotkey manager per process (macOS refuses a
//! second one), and tests of a file run in parallel threads of one process.

use daimos_platform::hotkey::{Binding, GlobalShortcutHotkey, Hotkey, HotkeyError};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

fn has_display() -> bool {
    !cfg!(target_os = "linux") || std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
}

#[test]
fn registers_unregisters_and_rejects_an_invalid_binding() {
    if !has_display() {
        eprintln!("no DISPLAY, skipping");
        return;
    }
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app");
    let handle = app.handle().clone();
    let hotkey = GlobalShortcutHotkey::new(handle.clone());

    let binding = Binding::new("Ctrl+Alt+Shift+F11");
    let registration = hotkey.register(&binding).expect("register");
    assert_eq!(registration.binding, binding);
    assert!(handle.global_shortcut().is_registered("Ctrl+Alt+Shift+F11"));

    hotkey.unregister(&registration).expect("unregister");
    assert!(!handle.global_shortcut().is_registered("Ctrl+Alt+Shift+F11"));

    let err = hotkey.register(&Binding::new("NotAKey+Banana")).unwrap_err();
    assert!(
        matches!(err, HotkeyError::Invalid { binding, .. } if binding.as_str() == "NotAKey+Banana")
    );
}
