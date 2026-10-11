//! Which window system the process runs under.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Session {
    X11,
    Wayland,
    Windows,
    MacOs,
}

impl Session {
    /// Detects the session of this process.
    pub fn detect() -> Self {
        Self::detect_with(|key| std::env::var(key).ok())
    }

    /// Same as [`Session::detect`] with the environment injected, so it can be tested.
    pub fn detect_with(env: impl Fn(&str) -> Option<String>) -> Self {
        if cfg!(target_os = "windows") {
            return Self::Windows;
        }
        if cfg!(target_os = "macos") {
            return Self::MacOs;
        }
        let set = |key| env(key).is_some_and(|v| !v.is_empty());
        let session_type = env("XDG_SESSION_TYPE").unwrap_or_default().to_ascii_lowercase();
        if session_type == "wayland" || (session_type.is_empty() && set("WAYLAND_DISPLAY")) {
            Self::Wayland
        } else {
            Self::X11
        }
    }

    /// Name of the compositor or desktop, for error messages and the capability list.
    pub fn desktop(env: impl Fn(&str) -> Option<String>) -> String {
        env("XDG_CURRENT_DESKTOP").filter(|v| !v.is_empty()).unwrap_or_else(|| "unknown".into())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn session_type_wayland_wins() {
        let e = env(&[("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0")]);
        assert_eq!(Session::detect_with(e), Session::Wayland);
    }

    #[test]
    fn session_type_x11_wins_over_a_stray_wayland_display() {
        let e = env(&[("XDG_SESSION_TYPE", "x11"), ("WAYLAND_DISPLAY", "wayland-0")]);
        assert_eq!(Session::detect_with(e), Session::X11);
    }

    #[test]
    fn without_session_type_the_wayland_display_decides() {
        assert_eq!(
            Session::detect_with(env(&[("WAYLAND_DISPLAY", "wayland-1")])),
            Session::Wayland
        );
        assert_eq!(Session::detect_with(env(&[("DISPLAY", ":1")])), Session::X11);
    }

    #[test]
    fn desktop_defaults_to_unknown() {
        assert_eq!(Session::desktop(env(&[])), "unknown");
        assert_eq!(Session::desktop(env(&[("XDG_CURRENT_DESKTOP", "KDE")])), "KDE");
    }
}
