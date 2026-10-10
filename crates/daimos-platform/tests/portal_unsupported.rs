//! With only `xdg-desktop-portal-wlr` (sway, as in the headless CI job) there is no
//! `GlobalShortcuts` interface: the backend must report `Unsupported` and the process keeps
//! running. The CI runner sets `DAIMOS_EXPECT_NO_PORTAL=1`; everywhere else the test is skipped,
//! because KDE and GNOME do implement the portal (and would show a dialog).
#![cfg(target_os = "linux")]

use daimos_platform::hotkey::{Binding, Hotkey, HotkeyError, PortalHotkey};

#[test]
fn a_compositor_without_the_portal_is_reported_as_unsupported() {
    if std::env::var_os("DAIMOS_EXPECT_NO_PORTAL").is_none() {
        eprintln!("DAIMOS_EXPECT_NO_PORTAL not set, skipping");
        return;
    }
    let hotkey = PortalHotkey::new("sway".into());
    let err = hotkey.register(&Binding::new("Alt+Space")).unwrap_err();
    assert_eq!(err, HotkeyError::Unsupported { compositor: "sway".into() });
    // The backend is still usable afterwards: a second attempt gives the same answer.
    assert!(matches!(
        hotkey.register(&Binding::new("Alt+Space")),
        Err(HotkeyError::Unsupported { .. })
    ));
}
