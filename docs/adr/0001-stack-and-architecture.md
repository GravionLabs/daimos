# ADR 0001 — Stack and architecture

## Status

Accepted

## Context

A daimos must open in well under 50 ms, stay resident for days without growing, register global
hotkeys on four window systems (X11, Wayland, Win32, AppKit), show a window without stealing focus,
and run third-party extensions. The team already ships a Tauri 2 + Rust + Angular desktop app
(MCP Studio) and a Tauri 2 + Angular app (Bench).

## Decision

- **Tauri 2** shell with a Rust core. The Rust side owns hotkeys, the window, indexing, matching,
  clipboard, and extensions. The webview only renders.
- **SolidJS** (Vite, TypeScript) for the frontend instead of Angular. The daimos window is one list
  and a few panes; the cost of Angular's bootstrap and bundle is paid on every cold start and
  buys nothing here. Solid's fine-grained reactivity keeps per-keystroke re-rendering cheap, and
  its bundle is small enough that the webview is warm in the time budget.
- **One resident process**: the Tauri process creates the window at start and only hides and shows
  it. A separate `daimos` CLI binary talks to it over a local socket.
- **Cargo workspace**: `daimos-core` (no Tauri dependency), `daimos-platform` (OS integration
  behind traits), `daimos-ext` (extension host), `daimos-cli`, `src-tauri`.
- **Matching**: `nucleo-matcher`. **Persistence**: SQLite via `rusqlite` with FTS5, synchronous,
  because the index is small and read on the hot path. Secrets in the OS keyring.
- **Bindings**: `specta` + `tauri-specta`, as in MCP Studio.
- **Platforms**: Linux (X11 and Wayland), Windows, macOS from the first milestone.

## Alternatives considered

- **Angular**: team consistency, but start-up cost and bundle size work against the budget. Can be
  reconsidered for a separate settings window if the UI grows.
- **Native Rust UI (egui, iced, gpui)**: fastest start-up, but every list, form, and Markdown detail
  is hand-built, and extension-provided views become a custom layout engine. Not worth it at this
  stage.
- **Electron**: memory and start-up are out of budget.
- **Qt (as Vicinae does)**: strong on Linux, but it moves the team off its Rust and web toolchain.

## Consequences

- The frontend is a new framework for the team; the UI is small enough that this is contained.
- The core runs headless, so benchmarks and the extension conformance suite run in plain `cargo`.
- Tauri's Linux backend is GTK3 + WebKitGTK; layer-shell and portal integration go through GTK and
  `ashpd`, not through Tauri's own window API.
