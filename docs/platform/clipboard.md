# Clipboard history

The history is **off by default** (settings → Clipboard history). Text and images are stored in
`clipboard.db` in the app data directory; equal content is stored once, the newest 500 entries are
kept besides the pinned ones, and images over 10 MB are not stored. Content that a password manager
marks as secret is never stored.

| Platform                            | How the clipboard is followed                                                                                                                                                         | Secret marker                                         |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| X11                                 | XFixes selection events, then `TARGETS` and the content (with `INCR`)                                                                                                                 | target `x-kde-passwordManagerHint`                    |
| Wayland (sway, Hyprland, niri, KDE) | `wlr-data-control`: every selection, whichever window has the focus                                                                                                                   | mime type `x-kde-passwordManagerHint`                 |
| Wayland (GNOME)                     | No data-control protocol, but Mutter mirrors the clipboard to Xwayland: the X11 backend follows it there (tried on GNOME 50: copies of Wayland-native apps arrive, without any focus) | same as X11                                           |
| Windows                             | `AddClipboardFormatListener` (`WM_CLIPBOARDUPDATE`); text and `PNG`                                                                                                                   | format `ExcludeClipboardContentFromMonitorProcessing` |
| macOS                               | `changeCount` polled every 250 ms; text and `public.png`                                                                                                                              | type `org.nspasteboard.ConcealedType`                 |

Not yet: images that are only on the clipboard as a bitmap (Windows `CF_DIB`, macOS TIFF). On
GNOME the recording needs Xwayland (GNOME starts it on demand; a session without it is `Unsupported`).

## The history view

Type `clipboard` to find the **Clipboard History** command, which lists everything, or type
`clip <text>` to search it (the fuzzy matcher runs over the text entries). Newest first, pinned
entries on top.

| Key                    | Action                                             |
| ---------------------- | -------------------------------------------------- |
| **Enter**              | Paste into the previous window (see `paste.md`)    |
| **Ctrl/Cmd+Enter**     | Copy only                                          |
| **Ctrl/Cmd+P**         | Pin or unpin                                       |
| **Ctrl/Cmd+Backspace** | Delete                                             |
| **Tab**                | Detail: the whole text, or the image with its size |

Images (up to 1 MB in the preview) can be pinned and deleted but not pasted yet.
