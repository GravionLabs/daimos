//! OS integration behind one trait per concern; one module per platform, selected with `cfg`.

pub mod hotkey;
pub mod platform;
pub mod session;
pub mod window;
#[cfg(windows)]
pub mod windows;
