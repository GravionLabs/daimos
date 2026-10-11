//! Pasters: they send the paste shortcut to the window that has the focus. One per platform;
//! [`paster`] picks the one for this session.

use std::sync::Arc;

use daimos_core::paste::{PasteSupport, Paster};

use crate::session::Session;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "linux")]
pub mod wayland;
#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "linux")]
pub mod x11;

/// The paster for `session`.
pub fn paster(session: Session) -> Arc<dyn Paster> {
    match session {
        #[cfg(target_os = "linux")]
        Session::X11 => Arc::new(x11::XTestPaster),
        #[cfg(target_os = "linux")]
        Session::Wayland => Arc::new(wayland::WaylandPaster::from_env()),
        #[cfg(windows)]
        Session::Windows => Arc::new(windows::SendInputPaster),
        #[cfg(target_os = "macos")]
        Session::MacOs => Arc::new(macos::CgEventPaster),
        #[allow(unreachable_patterns)]
        _ => Arc::new(Unsupported),
    }
}

struct Unsupported;

impl Paster for Unsupported {
    fn id(&self) -> &str {
        "none"
    }

    fn support(&self) -> PasteSupport {
        PasteSupport::Unsupported("pasting is not available in this session".into())
    }

    fn paste(&self) -> std::io::Result<()> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no paster"))
    }
}
