//! X11, Windows, and macOS: `tauri-plugin-global-shortcut` (XGrabKey, `RegisterHotKey`, Carbon).

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use super::{Binding, Hotkey, HotkeyError, Press, PressBus, Registration};

pub struct GlobalShortcutHotkey<R: Runtime> {
    app: AppHandle<R>,
    bus: Arc<PressBus>,
}

impl<R: Runtime> GlobalShortcutHotkey<R> {
    /// The app must have the `tauri_plugin_global_shortcut` plugin installed.
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app, bus: Arc::default() }
    }
}

fn parse(binding: &Binding) -> Result<Shortcut, HotkeyError> {
    binding
        .as_str()
        .parse::<Shortcut>()
        .map_err(|e| HotkeyError::Invalid { binding: binding.clone(), detail: e.to_string() })
}

impl<R: Runtime> Hotkey for GlobalShortcutHotkey<R> {
    fn register(&self, binding: &Binding) -> Result<Registration, HotkeyError> {
        let shortcut = parse(binding)?;
        let bus = Arc::clone(&self.bus);
        let pressed = binding.clone();
        self.app
            .global_shortcut()
            .on_shortcut(shortcut, move |_, _, event| {
                if event.state == ShortcutState::Pressed {
                    bus.publish(&Press { binding: pressed.clone() });
                }
            })
            // The binding parsed, so what is left is the OS saying no.
            .map_err(|e| HotkeyError::AlreadyTaken {
                binding: binding.clone(),
                detail: e.to_string(),
            })?;
        Ok(Registration { binding: binding.clone() })
    }

    fn unregister(&self, registration: &Registration) -> Result<(), HotkeyError> {
        let shortcut = parse(&registration.binding)?;
        self.app
            .global_shortcut()
            .unregister(shortcut)
            .map_err(|e| HotkeyError::Backend(e.to_string()))
    }

    fn presses(&self) -> Receiver<Press> {
        self.bus.subscribe()
    }
}
