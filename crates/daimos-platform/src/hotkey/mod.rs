//! The global hotkey that toggles the window.
//!
//! One trait, one backend per window system. X11, Windows, and macOS share the backend built on
//! `tauri-plugin-global-shortcut`; Wayland has its own (the XDG GlobalShortcuts portal).

mod global_shortcut;
mod service;

use std::fmt;
use std::sync::mpsc::Receiver;

use serde::Serialize;
use tauri::{AppHandle, Runtime};

pub use global_shortcut::GlobalShortcutHotkey;
pub use service::HotkeyService;

use crate::session::Session;

/// An accelerator such as `Alt+Space` or `Ctrl+Shift+P`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Binding(String);

impl Binding {
    pub fn new(accelerator: impl Into<String>) -> Self {
        Self(accelerator.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Proof that a binding is registered; hand it back to [`Hotkey::unregister`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub binding: Binding,
}

/// The hotkey was pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Press {
    pub binding: Binding,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HotkeyError {
    /// The OS refused the binding, usually because another program owns it.
    #[error("the binding {binding} is already taken: {detail}")]
    AlreadyTaken { binding: Binding, detail: String },
    #[error("{binding} is not a valid binding: {detail}")]
    Invalid { binding: Binding, detail: String },
    /// The compositor offers no way to register a global shortcut (Wayland without the portal).
    #[error("global shortcuts are not supported on {compositor}; bind `daimos toggle` instead")]
    Unsupported { compositor: String },
    #[error("hotkey backend failed: {0}")]
    Backend(String),
}

pub trait Hotkey: Send + Sync {
    fn register(&self, binding: &Binding) -> Result<Registration, HotkeyError>;
    fn unregister(&self, registration: &Registration) -> Result<(), HotkeyError>;
    /// A new stream of presses; every subscriber sees every press from now on.
    fn presses(&self) -> Receiver<Press>;
}

/// Picks the backend for the current window system.
pub fn detect<R: Runtime>(app: &AppHandle<R>) -> Box<dyn Hotkey> {
    detect_for(app, Session::detect())
}

/// [`detect`] for a given session.
pub fn detect_for<R: Runtime>(app: &AppHandle<R>, session: Session) -> Box<dyn Hotkey> {
    match session {
        // Wayland has no global grab; the portal backend lands with the XDG GlobalShortcuts PBI.
        // Until then the compositor binding for `daimos toggle` is the way in.
        Session::Wayland => {
            Box::new(Unsupported { compositor: Session::desktop(|k| std::env::var(k).ok()) })
        }
        Session::X11 | Session::Windows | Session::MacOs => {
            Box::new(GlobalShortcutHotkey::new(app.clone()))
        }
    }
}

/// Backend for a session that cannot register a hotkey.
struct Unsupported {
    compositor: String,
}

impl Hotkey for Unsupported {
    fn register(&self, _: &Binding) -> Result<Registration, HotkeyError> {
        Err(HotkeyError::Unsupported { compositor: self.compositor.clone() })
    }

    fn unregister(&self, _: &Registration) -> Result<(), HotkeyError> {
        Ok(())
    }

    fn presses(&self) -> Receiver<Press> {
        std::sync::mpsc::channel().1
    }
}

/// Fan-out of presses to every subscriber; dropped receivers are forgotten on the next press.
#[derive(Default)]
pub(crate) struct PressBus {
    senders: std::sync::Mutex<Vec<std::sync::mpsc::Sender<Press>>>,
}

impl PressBus {
    pub(crate) fn subscribe(&self) -> Receiver<Press> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.senders.lock().expect("press bus lock").push(tx);
        rx
    }

    pub(crate) fn publish(&self, press: &Press) {
        self.senders.lock().expect("press bus lock").retain(|tx| tx.send(press.clone()).is_ok());
    }
}
