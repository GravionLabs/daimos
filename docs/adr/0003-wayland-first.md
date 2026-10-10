# ADR 0003 — Wayland first

## Status

Accepted

## Context

Most Linux daimoss were written for X11 and treat Wayland as a port. On Wayland there is no
protocol for an application to grab a global key, no way to position a toplevel window, and no
way to read the clipboard without focus. Fedora, Ubuntu, and KDE default to Wayland; GNOME is
removing X11 session support. A daimos that only works on X11 is a daimos for last decade's
Linux.

## Decision

Wayland support is part of M0 and a condition of the M0 gate, alongside X11, Windows, and macOS.

- **Hotkey**: the XDG Desktop Portal `GlobalShortcuts` interface via `ashpd`, which works on KDE
  Plasma, GNOME 48 and later, and Hyprland. The user binds the shortcut once in the portal dialog;
  the binding survives restarts. Where the portal is missing (Sway and other wlroots compositors
  without a portal backend for it), the documented path is a compositor keybinding that runs
  `daimos toggle`, which signals the resident process over its socket. The settings page detects
  which path is active and shows the instructions for the current compositor.
- **Window**: `gtk-layer-shell` on compositors that implement `wlr-layer-shell` (wlroots, KDE), so
  the window is an overlay with keyboard focus and a fixed position. On GNOME, which does not
  implement layer-shell, the window is a normal toplevel; GNOME centers new windows, which is the
  wanted position anyway.
- **Clipboard history**: `wlr-data-control` through `wl-clipboard-rs` where available; on GNOME the
  history provider is limited to what the focused daimos window can read and says so.
- **Paste into the previous window**: `wtype` or `ydotool` if installed, otherwise the action
  degrades to copy with a toast.
- **CI**: a headless `sway` session runs the hotkey and window tests on every push.

## Alternatives considered

- **X11 first, Wayland later**: the hotkey and window code would be written twice, and the second
  version would be the one most Linux users run. Rejected.
- **XWayland for the daimos window**: gives X11 semantics, but global key grabs still do not work
  under XWayland, and the window looks wrong on HiDPI. Rejected.

## Consequences

- Linux has two code paths for hotkey and window from the start; both are behind the
  `daimos-platform` traits and both are tested.
- Some Wayland capabilities degrade explicitly (paste, clipboard on GNOME); the UI always says
  which feature is limited and why.
- GTK and `ashpd` become direct dependencies of the Linux build, independent of Tauri's window
  API.
