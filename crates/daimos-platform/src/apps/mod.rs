//! Application sources: one module per OS, selected with `cfg`.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;
