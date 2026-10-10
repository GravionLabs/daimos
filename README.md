# Daimos

> Working name. See the [product spec](docs/specs/product-spec.md) for open decisions.

A cross-platform command palette for the desktop: one hotkey, one window, the same extensions on
Linux (X11 and Wayland), Windows, and macOS. Built with Tauri 2, a Rust core, and SolidJS.

Status: M0 Foundation in progress (scaffold, hotkey, resident window).

## Build and run

Requirements: Rust (the toolchain in `rust-toolchain.toml` is installed by `rustup`), Node.js 24 and
pnpm (`corepack enable`), and the Tauri prerequisites of your platform.

| Platform | Prerequisites |
| -------- | ------------- |
| Linux    | `sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev pkg-config build-essential` (X11 and Wayland) |
| Windows  | Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 11) |
| macOS    | Xcode command line tools (`xcode-select --install`) |

```sh
pnpm install
pnpm tauri dev        # the app with a hot-reloading UI
cargo build --workspace
```

The Rust crates embed `apps/ui/dist`, so build the UI once (`pnpm --filter @daimos/ui build`) before
a release build of `src-tauri`; `cargo clippy` and `cargo test` work without it.

## Verify

```sh
cargo fmt --all --check && cargo clippy --workspace -- -D warnings && cargo test --workspace
pnpm lint && pnpm test
```

## Documents

- [Product spec](docs/specs/product-spec.md) — audience, scope, milestones, budget, risks, decisions
- [Architecture](docs/specs/architecture.md) — process model, crates, query pipeline, platform layer
- [Extensions](docs/specs/extensions.md) — manifest, script protocol, wasm interface, conformance
- [ADR 0001 — Stack and architecture](docs/adr/0001-stack-and-architecture.md)
- [ADR 0002 — Extension model](docs/adr/0002-extension-model.md)
- [ADR 0003 — Wayland first](docs/adr/0003-wayland-first.md)
