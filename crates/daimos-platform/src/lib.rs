//! OS integration behind one trait per concern; one module per platform, selected with `cfg`.

pub mod hotkey;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod platform;
pub mod session;
pub mod window;
#[cfg(windows)]
pub mod windows;
