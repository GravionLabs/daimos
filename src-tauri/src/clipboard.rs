//! The clipboard history service: it follows the clipboard while the setting is on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use daimos_core::clipboard::{
    Capture, ClipboardBackend, ClipboardHistory, ClipboardSupport, Limits,
};
use daimos_core::settings::ClipboardSettings;

pub struct ClipboardService {
    backend: Arc<dyn ClipboardBackend>,
    history: Arc<ClipboardHistory>,
    capture: Mutex<Option<Capture>>,
    /// Whether the capture is on, for the history list.
    enabled: Arc<AtomicBool>,
}

pub fn limits(settings: &ClipboardSettings) -> Limits {
    Limits {
        max_entries: settings.max_entries,
        max_image_bytes: settings.max_image_mb.saturating_mul(1024 * 1024),
        ..Limits::default()
    }
}

impl ClipboardService {
    pub fn new(backend: Arc<dyn ClipboardBackend>, history: Arc<ClipboardHistory>) -> Self {
        Self {
            backend,
            history,
            capture: Mutex::new(None),
            enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The provider that lists the history and runs its actions.
    pub fn provider(&self) -> Arc<dyn daimos_core::Provider> {
        Arc::new(daimos_core::clipboard::ClipboardProvider::new(
            Arc::clone(&self.history),
            Arc::clone(&self.enabled),
        ))
    }

    /// Makes the capture match `settings`: starts it, stops it, or only updates the limits.
    pub fn apply(&self, settings: &ClipboardSettings) {
        self.history.set_limits(limits(settings));
        self.enabled.store(settings.enabled, Ordering::SeqCst);
        let mut capture = self.capture.lock().unwrap_or_else(|e| e.into_inner());
        match (settings.enabled, capture.is_some()) {
            (true, false) => {
                match Capture::start(Arc::clone(&self.backend), Arc::clone(&self.history)) {
                    Ok(started) => *capture = Some(started),
                    Err(error) => log::warn!("clipboard history not started: {error}"),
                }
            }
            (false, true) => {
                if let Some(running) = capture.take() {
                    running.stop();
                }
            }
            _ => {}
        }
    }

    pub fn support(&self) -> ClipboardSupport {
        self.backend.support()
    }

    pub fn stop(&self) {
        if let Some(running) = self.capture.lock().unwrap_or_else(|e| e.into_inner()).take() {
            running.stop();
        }
    }
}
