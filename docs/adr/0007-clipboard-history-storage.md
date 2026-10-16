# ADR 0007 — Clipboard history storage and the GNOME fallback

## Status

Accepted

## Context

Clipboard history (M3, v0.2 and v0.3) stores what the user copies and pastes it back into the
window that was active before Daimos opened. It is the feature with the most privacy exposure in
the product and the one where the platforms differ the most: X11 and the wlroots-style Wayland
compositors report every clipboard change, GNOME's Mutter has no data-control protocol, Windows and
macOS have their own listeners and their own "do not record" markers. ADR 0003 assumed that on
GNOME the history would be limited to what the focused Daimos window can read.

## Decision

- **Off by default.** Recording starts only after the user turns the history on in the settings.
- **Local SQLite, bounded.** Entries go to `clipboard.db` in the app data directory. Equal content
  is stored once; the newest 500 entries are kept besides the pinned ones; texts over 1 MB and
  images over 10 MB are not stored. Images are stored as PNG and loaded on demand.
- **Concealed content is never stored.** A backend reports the platform's password-manager marker
  (`x-kde-passwordManagerHint` on X11 and Wayland, `ExcludeClipboardContentFromMonitorProcessing`
  on Windows, `org.nspasteboard.ConcealedType` on macOS) and the service drops the snapshot.
- **One backend per window system behind `ClipboardBackend` in `daimos-core`,** so the history
  itself is tested headless with a mock: `x11rb` with XFixes selection events on X11,
  `wlr-data-control` through `wayland-client` on Wayland, `AddClipboardFormatListener` on Windows,
  `changeCount` polling every 250 ms on macOS. A backend reports its support level (`Full`,
  `Limited`, `Unsupported`) and the settings show it.
- **GNOME follows the clipboard through Xwayland.** Mutter mirrors the Wayland clipboard to
  Xwayland, so on a compositor without `wlr-data-control` the Wayland backend hands over to the X11
  backend on the Xwayland display. Copies from Wayland-native apps arrive without any focus
  (verified on GNOME 50). This replaces the "focused window only" plan of ADR 0003. A session
  without Xwayland reports `Unsupported`.
- **Paste** puts the text on the clipboard, hides the window, waits for the previous window to take
  the focus and sends the paste shortcut: XTest on X11, `wtype` or `ydotool` on Wayland (GNOME:
  `ydotool` only), `SendInput` on Windows, `CGEvent` on macOS. Where that is impossible the action
  degrades to copy with a toast that says why.

## Alternatives considered

- **Record only while the Daimos window is focused on GNOME** (ADR 0003). Superseded: it would have
  missed almost every copy, and the Xwayland mirror gives the full history without an extension.
- **A GNOME Shell extension that forwards the clipboard.** Rejected: a second install step, breaks
  with every GNOME release, and the Xwayland path does not need it.
- **Store images as files next to the database.** Rejected: one file keeps deletion, limits and
  deduplication in one place, and the 10 MB cap keeps the database small.
- **Keep the history in memory only.** Rejected: users expect the history to survive a restart;
  the off-by-default switch and the concealed markers address the privacy concern instead.

## Consequences

- `docs/platform/clipboard.md` and `docs/platform/paste.md` document the per-platform behavior of
  this decision and are updated with it.
- Bitmap-only images (Windows `CF_DIB`, macOS TIFF), restoring the previous clipboard after a
  paste, Ctrl+Shift+V for terminals and pasting images are not covered yet and are tracked as
  follow-ups of the clipboard feature.
- GNOME users need Xwayland for the history; GNOME starts it on demand, and a pure-Wayland session
  is reported, not silently empty.
