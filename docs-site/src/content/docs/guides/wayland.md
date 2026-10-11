---
title: Wayland setup
description: The global hotkey and the window on Wayland compositors.
---

Wayland has no global key grab: an application cannot listen for a key combination while another
window has the focus. Daimos asks the compositor through the XDG GlobalShortcuts portal, and falls
back to a command you bind yourself.

## Hotkey

| Desktop                   | Result                                                                                              |
| ------------------------- | --------------------------------------------------------------------------------------------------- |
| KDE Plasma 6              | Works. The first start shows a dialog; change the shortcut in System Settings → Shortcuts.          |
| GNOME 48 and later        | Works. The first start shows a dialog; GNOME manages the shortcut.                                  |
| Hyprland                  | Works with `xdg-desktop-portal-hyprland`.                                                           |
| sway, niri, other wlroots | No portal interface. Bind `daimos toggle` yourself (below).                                         |
| GNOME before 48           | No portal interface. Same fallback.                                                                 |

The portal remembers the binding, so later starts show no dialog. The trigger in the settings is
only a suggestion: if you pick another one in the dialog, that one wins.

## Fallback: bind `daimos toggle`

Anything that can run a command on a key can drive Daimos:

```
# sway (~/.config/sway/config)
bindsym Mod1+space exec daimos toggle

# Hyprland (~/.config/hypr/hyprland.conf)
bind = ALT, space, exec, daimos toggle

# niri (~/.config/niri/config.kdl)
binds {
    Alt+Space { spawn "daimos" "toggle"; }
}
```

## Window

| Compositor                       | Window                                                                           |
| -------------------------------- | -------------------------------------------------------------------------------- |
| sway, Hyprland, niri, KDE Plasma | A `wlr-layer-shell` surface on the overlay layer; the compositor centers it.     |
| GNOME                            | A normal window; GNOME centers new windows itself (it has no layer-shell).       |

In both modes the window hides when it loses the focus.
