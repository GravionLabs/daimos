//! Clipboard backends, one per window system. [`backend`] picks the one for this session.

use std::sync::Arc;

use daimos_core::clipboard::ClipboardBackend;

use crate::session::Session;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod x11;

/// The backend for `session`.
pub fn backend(session: Session) -> Arc<dyn ClipboardBackend> {
    match session {
        #[cfg(target_os = "linux")]
        Session::X11 => Arc::new(x11::X11Clipboard),
        #[cfg(windows)]
        Session::Windows => Arc::new(windows::WindowsClipboard),
        #[cfg(target_os = "macos")]
        Session::MacOs => Arc::new(macos::MacClipboard),
        #[cfg(target_os = "linux")]
        Session::Wayland => Arc::new(wayland::WaylandClipboard),
        _ => Arc::new(Unsupported("no clipboard backend for this session")),
    }
}

struct Unsupported(&'static str);

impl ClipboardBackend for Unsupported {
    fn id(&self) -> &str {
        "none"
    }

    fn support(&self) -> daimos_core::clipboard::ClipboardSupport {
        daimos_core::clipboard::ClipboardSupport::Unsupported(self.0.into())
    }

    fn watch(
        &self,
        _sink: daimos_core::clipboard::ClipboardSink,
        _stop: Arc<std::sync::atomic::AtomicBool>,
    ) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, self.0))
    }
}
