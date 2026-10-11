# Changelog

All notable changes to GravionLabs/daimos are documented here.
Versioning follows [Semantic Versioning](https://semver.org/) and GitVersion (GitVersion).

---

## [0.2.0-beta.25] — 2026-10-11

### Bug Fixes
- Measure memory as PSS, footprint or private working set instead of RSS (#103) (#106)
- Hold show and query until the UI listens and replay them (#105) (#107)
- Offer a changed hotkey to the Wayland portal and report what it kept (#104) (#108)
- Settings button, label spacing and search outline (#122)
- Next-version 0.2.0 so release/0.2 is numbered 0.2.0-beta.n


### Documentation
- Docs site with Starlight, deployed to GitHub Pages (#114) (#75)
- Update CHANGELOG.md for v0.2.0
- A minor release needs +semver: minor in a commit message


### Features
- Reveal action for applications on Linux and Windows (#111) (#112)
- Clipboard history store, capture service and backend trait (#116) (#60)
- Linux clipboard backends for X11 and wlr-data-control (#117) (#60)
- Windows and macOS clipboard backends (#118) (#60)
- Clipboard history setting and app wiring (#119) (#60)


## [0.1.0-beta.28] — 2026-10-11

### Bug Fixes
- Repeat the wake-up connect when the IPC server stops (#43)
- Resolve the terminal emulator against the source's search path (#39)
- The configured provider weight reaches the result batches (#50)


### Documentation
- Product spec, architecture, extension model and ADRs for Daimos
- Update CHANGELOG.md for v0.1.0


### Features
- Foundation, hotkey, and window (#1) (#76)
- Single instance; the second-listener test is Unix only
- Non-activating NSPanel on macOS (#35)
- Tool window and foreground after the hotkey on Windows (#36)
- Layer-shell overlay on Wayland where supported, toplevel on GNOME (#37)
- Tray icon and autostart (#38)
- Daimos CLI with toggle, show, hide, and quit over a local socket (#33)
- Resident window that is created once and only hidden and shown (#34)
- Hotkey on Wayland via the XDG GlobalShortcuts portal (#32)
- Hotkey trait with X11, Windows, and macOS backends (#31)
- Provider trait and query engine with cancellation and streamed results (#43) (#79)
- Fuzzy matcher with nucleo and frecency ranking (#44) (#80)
- Performance harness with the budget as a CI check (#45) (#83)
- AppSource for Linux from .desktop entries with icon theme lookup (#39) (#85)
- AppSource for Windows from shell:AppsFolder with icons (#40) (#88)
- Index storage in SQLite with icon cache and change watching (#42)
- AppSource for macOS from .app bundles with icons (#41) (#89)
- Search field, result list, and keyboard model (#46)
- Action panel and detail pane (#47)
- Inline math, unit conversion, and currency with copy result (#49) (#94)
- Light and dark theme with the OS accent and reduced motion (#48) (#93)
- Settings page for hotkey, theme, providers, aliases, and platform capabilities (#50) (#97)



