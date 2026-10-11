# Paste into the previous window

The `paste` action puts the text on the clipboard (the web view does that, so it works on every
compositor), hides the window, waits 150 ms for the previous window to take the focus and sends the
paste shortcut to it. Where the shortcut cannot be sent the window stays open, the text is copied,
and a toast says why. The settings list the state under "Paste into the previous window".

| Platform                            | How the shortcut is sent                                               | Degrades to copy when                                                      |
| ----------------------------------- | ---------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| X11                                 | XTest fake key events (Ctrl+V)                                         | the server has no XTest extension                                          |
| Wayland (sway, Hyprland, niri, KDE) | `wtype` (virtual keyboard protocol), else `ydotool` (needs its daemon) | neither tool is installed                                                  |
| Wayland (GNOME)                     | `ydotool` only: GNOME has no virtual keyboard protocol for `wtype`     | `ydotool` is not installed or its daemon is not running                    |
| Windows                             | `SendInput` Ctrl+V                                                     | never (a protected window ignores it; nothing is reported)                 |
| macOS                               | `CGEvent` Cmd+V                                                        | the Accessibility permission is missing (the toast says where to grant it) |

Not yet: restoring the previous clipboard content after the paste, Ctrl+Shift+V for terminals, and
pasting images.
