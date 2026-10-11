//! What this system can do, in words for the settings page: each capability is `available`,
//! `limited` or `unavailable`, with the reason, and the compositor instructions on Wayland.

use serde::{Deserialize, Serialize};
use specta::Type;

use daimos_platform::hotkey::{Binding, trigger_matches};

use crate::bindings::HotkeyStatus;
use daimos_core::paste::PasteSupport;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Available,
    Limited,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Capability {
    pub id: String,
    pub label: String,
    pub level: Level,
    pub reason: String,
    /// Steps the user can take, shown below the reason (a compositor configuration, for example).
    pub instructions: Option<String>,
}

fn capability(id: &str, label: &str, level: Level, reason: impl Into<String>) -> Capability {
    Capability {
        id: id.into(),
        label: label.into(),
        level,
        reason: reason.into(),
        instructions: None,
    }
}

/// The compositor configuration that binds `daimos toggle` where the portal is missing.
pub const WAYLAND_INSTRUCTIONS: &str = "\
Bind the command `daimos toggle` to a key in your compositor:
  sway       bindsym Mod1+space exec daimos toggle
  Hyprland   bind = ALT, space, exec, daimos toggle
  niri       Alt+Space { spawn \"daimos\" \"toggle\"; }
  GNOME      Settings > Keyboard > Custom Shortcuts: command `daimos toggle`
  KDE Plasma System Settings > Shortcuts > Add Command: `daimos toggle`";

/// The status after `binding` was registered. `reported` is the trigger the system says it uses:
/// on Wayland the compositor owns the shortcut and may have kept another one than asked for, and
/// then the status names the real one and says where to change it.
pub fn bound_status(session: &str, binding: &Binding, reported: Option<String>) -> HotkeyStatus {
    let (shown, message) = match reported {
        Some(trigger) if !trigger_matches(binding, &trigger) => (
            trigger.clone(),
            Some(format!(
                "Your desktop keeps {trigger} for Daimos instead of {binding}: on Wayland the compositor owns the shortcut. Change it in the system settings (KDE: Shortcuts, GNOME: Keyboard)."
            )),
        ),
        _ => (binding.to_string(), None),
    };
    HotkeyStatus { state: "active".into(), session: session.into(), binding: Some(shown), message }
}

/// The capabilities for a session (`x11`, `wayland`, `windows`, `macos`), the window mode
/// (`layer_shell` or `toplevel`) and the state of the hotkey.
pub fn capabilities(session: &str, window_mode: &str, hotkey: &HotkeyStatus) -> Vec<Capability> {
    vec![hotkey_capability(session, hotkey), window_capability(session, window_mode), clipboard()]
}

fn hotkey_capability(session: &str, hotkey: &HotkeyStatus) -> Capability {
    let how = match session {
        "wayland" => "the XDG GlobalShortcuts portal",
        "x11" => "a global key grab (XGrabKey)",
        "windows" => "RegisterHotKey",
        _ => "the system's global shortcut API",
    };
    let message = hotkey.message.clone().unwrap_or_default();
    let mut capability = match hotkey.state.as_str() {
        "active" => capability(
            "hotkey",
            "Global hotkey",
            Level::Available,
            format!(
                "Registered through {how}{}.{}",
                hotkey.binding.as_ref().map_or(String::new(), |b| format!(" as {b}")),
                if message.is_empty() { String::new() } else { format!(" {message}") }
            ),
        ),
        "pending" => capability(
            "hotkey",
            "Global hotkey",
            Level::Limited,
            "Waiting for you to answer the system dialog that asks to allow the shortcut.",
        ),
        "unsupported" => capability(
            "hotkey",
            "Global hotkey",
            Level::Unavailable,
            if message.is_empty() {
                "This desktop has no way to register a global shortcut.".to_owned()
            } else {
                message
            },
        ),
        _ => capability(
            "hotkey",
            "Global hotkey",
            Level::Unavailable,
            if message.is_empty() {
                "The shortcut could not be registered.".to_owned()
            } else {
                message
            },
        ),
    };
    if session == "wayland" && matches!(hotkey.state.as_str(), "unsupported" | "failed") {
        capability.instructions = Some(WAYLAND_INSTRUCTIONS.to_owned());
    }
    capability
}

fn window_capability(session: &str, window_mode: &str) -> Capability {
    match (window_mode, session) {
        ("layer_shell", _) => capability(
            "window",
            "Palette window",
            Level::Available,
            "An overlay surface (wlr-layer-shell): it appears above other windows and takes the keyboard.",
        ),
        (_, "wayland") => capability(
            "window",
            "Palette window",
            Level::Limited,
            "A normal window: this compositor has no layer-shell (GNOME), so it decides where the window goes and may not float above full-screen windows.",
        ),
        (_, "macos") => capability(
            "window",
            "Palette window",
            Level::Available,
            "A non-activating panel that joins every Space and shows over full-screen apps.",
        ),
        (_, "windows") => capability(
            "window",
            "Palette window",
            Level::Available,
            "A tool window: no taskbar button, in front of other windows after the hotkey.",
        ),
        _ => capability(
            "window",
            "Palette window",
            Level::Available,
            "A window without decoration, centered on the monitor with the cursor.",
        ),
    }
}

fn clipboard() -> Capability {
    capability(
        "clipboard",
        "Copy to clipboard",
        Level::Available,
        "Results are copied through the web view's clipboard.",
    )
}

/// Whether the shortcut that pastes into the previous window can be sent.
pub fn paste_capability(support: &PasteSupport) -> Capability {
    match support {
        PasteSupport::Full => capability(
            "paste",
            "Paste into the previous window",
            Level::Available,
            "The window you came from gets the paste shortcut.",
        ),
        PasteSupport::Limited(reason) => {
            capability("paste", "Paste into the previous window", Level::Limited, reason.clone())
        }
        PasteSupport::Unsupported(reason) => capability(
            "paste",
            "Paste into the previous window",
            Level::Unavailable,
            format!("{reason}. The text is copied instead."),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: &str, message: Option<&str>) -> HotkeyStatus {
        HotkeyStatus {
            state: state.into(),
            session: "x".into(),
            binding: Some("Alt+Space".into()),
            message: message.map(Into::into),
        }
    }

    fn find<'a>(all: &'a [Capability], id: &str) -> &'a Capability {
        all.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn an_active_hotkey_names_the_mechanism_and_the_binding() {
        let all = capabilities("x11", "toplevel", &status("active", None));
        let hotkey = find(&all, "hotkey");
        assert_eq!(hotkey.level, Level::Available);
        assert!(
            hotkey.reason.contains("XGrabKey") && hotkey.reason.contains("Alt+Space"),
            "{}",
            hotkey.reason
        );
        assert!(hotkey.instructions.is_none());
    }

    #[test]
    fn a_trigger_the_desktop_kept_is_named_and_explained() {
        let asked = Binding::new("Alt+Space");
        // The same keys, written the way a portal words them: nothing to say.
        for same in ["Alt+Space", "<Alt>space", "Alt + Space", "ALT+space"] {
            let status = bound_status("wayland", &asked, Some(same.into()));
            assert_eq!(status.message, None, "{same}");
            assert_eq!(status.binding.as_deref(), Some("Alt+Space"));
        }
        assert_eq!(bound_status("x11", &asked, None).message, None);

        let kept = bound_status("wayland", &asked, Some("Meta+K".into()));
        assert_eq!(kept.state, "active");
        assert_eq!(kept.binding.as_deref(), Some("Meta+K"));
        let message = kept.message.clone().unwrap();
        assert!(
            message.contains("Meta+K")
                && message.contains("Alt+Space")
                && message.contains("system settings"),
            "{message}"
        );

        // The settings page shows it as part of the reason.
        let all = capabilities("wayland", "toplevel", &kept);
        let reason = &find(&all, "hotkey").reason;
        assert!(
            reason.contains("Registered through the XDG GlobalShortcuts portal")
                && reason.contains("keeps Meta+K"),
            "{reason}"
        );
        assert_eq!(find(&all, "hotkey").level, Level::Available);
    }

    #[test]
    fn a_pending_portal_dialog_is_limited() {
        let hotkey = capabilities("wayland", "toplevel", &status("pending", None));
        assert_eq!(find(&hotkey, "hotkey").level, Level::Limited);
    }

    #[test]
    fn wayland_without_the_portal_explains_the_fallback() {
        let all = capabilities(
            "wayland",
            "layer_shell",
            &status("unsupported", Some("no GlobalShortcuts portal")),
        );
        let hotkey = find(&all, "hotkey");
        assert_eq!(hotkey.level, Level::Unavailable);
        assert_eq!(hotkey.reason, "no GlobalShortcuts portal");
        let instructions = hotkey.instructions.as_deref().unwrap();
        for compositor in ["sway", "Hyprland", "niri", "GNOME", "KDE"] {
            assert!(instructions.contains(compositor), "{compositor}");
        }
        assert!(instructions.contains("daimos toggle"));
    }

    #[test]
    fn a_failed_hotkey_elsewhere_has_no_compositor_instructions() {
        let all = capabilities(
            "windows",
            "toplevel",
            &status("failed", Some("taken by another program")),
        );
        assert_eq!(find(&all, "hotkey").instructions, None);
        assert_eq!(find(&all, "hotkey").reason, "taken by another program");
    }

    #[test]
    fn the_window_is_limited_only_on_a_wayland_toplevel() {
        let level = |session, mode| {
            find(&capabilities(session, mode, &status("active", None)), "window").level.clone()
        };
        assert_eq!(level("wayland", "layer_shell"), Level::Available);
        assert_eq!(level("wayland", "toplevel"), Level::Limited);
        assert_eq!(level("x11", "toplevel"), Level::Available);
        assert_eq!(level("windows", "toplevel"), Level::Available);
        assert_eq!(level("macos", "toplevel"), Level::Available);
    }

    #[test]
    fn copy_works_and_paste_follows_the_paster() {
        let all = capabilities("x11", "toplevel", &status("active", None));
        assert_eq!(find(&all, "clipboard").level, Level::Available);
        assert_eq!(paste_capability(&PasteSupport::Full).level, Level::Available);
        let limited = paste_capability(&PasteSupport::Limited("grant Accessibility".into()));
        assert_eq!(
            (limited.level, limited.reason.as_str()),
            (Level::Limited, "grant Accessibility")
        );
        let none = paste_capability(&PasteSupport::Unsupported("install wtype".into()));
        assert_eq!(none.level, Level::Unavailable);
        assert!(none.reason.starts_with("install wtype"));
    }
}
