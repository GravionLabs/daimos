# Clipboard history

The history is **off by default** (settings → Clipboard history). Text and images are stored in
`clipboard.db` in the app data directory; equal content is stored once, the newest 500 entries are
kept besides the pinned ones, and images over 10 MB are not stored. Content that a password manager
marks as secret is never stored.

| Platform                            | How the clipboard is followed                                            | Secret marker                                         |
| ----------------------------------- | ------------------------------------------------------------------------ | ----------------------------------------------------- |
| X11                                 | XFixes selection events, then `TARGETS` and the content (with `INCR`)    | target `x-kde-passwordManagerHint`                    |
| Wayland (sway, Hyprland, niri, KDE) | `wlr-data-control`: every selection, whichever window has the focus      | mime type `x-kde-passwordManagerHint`                 |
| Wayland (GNOME)                     | No data-control protocol: reported as `Limited`, nothing is recorded yet | —                                                     |
| Windows                             | `AddClipboardFormatListener` (`WM_CLIPBOARDUPDATE`); text and `PNG`      | format `ExcludeClipboardContentFromMonitorProcessing` |
| macOS                               | `changeCount` polled every 250 ms; text and `public.png`                 | type `org.nspasteboard.ConcealedType`                 |

Not yet: images that are only on the clipboard as a bitmap (Windows `CF_DIB`, macOS TIFF), recording
on GNOME while the Daimos window is focused, and the history view with search, pin and paste
(#62, #61).
