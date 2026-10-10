# Wayland

Wayland has no global key grab: an application cannot listen for a key combination while another
window has the focus. Daimos therefore asks the compositor, and falls back to a command you bind
yourself. See [ADR 0003](../adr/0003-wayland-first.md).

## Hotkey: the XDG GlobalShortcuts portal

Daimos talks to `org.freedesktop.portal.GlobalShortcuts` (through `ashpd`). On the first start it
creates a session, binds a shortcut with the id `toggle` and the preferred trigger from the
settings (`Alt+Space` by default), and listens for `Activated`.

| Desktop                     | Result                                                                                                    |
| --------------------------- | --------------------------------------------------------------------------------------------------------- |
| KDE Plasma 6                | Works. The first start shows a dialog; the shortcut can be changed in System Settings → Shortcuts.         |
| GNOME 48 and later          | Works. The first start shows a dialog; the shortcut is managed by GNOME.                                   |
| Hyprland                    | Works with `xdg-desktop-portal-hyprland`.                                                                  |
| sway, niri, other wlroots   | No portal interface. The capability is reported as `unsupported`; use the CLI fallback below.              |
| GNOME before 48             | No portal interface. Same fallback.                                                                        |

The portal remembers the binding for the application id, so later starts find `toggle` already
bound and show no dialog. The preferred trigger is only a suggestion: if the user picks another
one in the dialog, that one wins.

## Fallback: bind `daimos toggle` in the compositor

Anything that can run a command on a key can drive Daimos through the CLI (`daimos toggle`, see the
CLI PBI). Examples for the compositors that lack the portal:

```
# sway (~/.config/sway/config)
bindsym Mod1+space exec daimos toggle

# Hyprland (~/.config/hypr/hyprland.conf), when not using the portal
bind = ALT, space, exec, daimos toggle

# niri (~/.config/niri/config.kdl)
binds {
    Alt+Space { spawn "daimos" "toggle"; }
}
```

## CI

The Ubuntu Wayland job runs the tests in a headless `sway` session with `xdg-desktop-portal` and
`xdg-desktop-portal-wlr` (`scripts/ci/run-in-sway.sh`). `xdg-desktop-portal-wlr` has no
GlobalShortcuts interface, so that job covers the detection path: the backend reports
`Unsupported` and the process keeps running. The KDE and GNOME path needs a real session; check it
by hand (see the manual checks of the Wayland hotkey PBI).
