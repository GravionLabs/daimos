# Releasing

A release is the merge of a release branch into `main`. Everything after the merge is automatic.

## Steps

1. **Cut the branch** from `main`: `release/<x.y>` (the `github-release` skill's `cut`). All work for
   the release is squash-merged into it through pull requests; pull requests into a release branch
   run the two Linux jobs of `ci.yml`. Add the `full-matrix` label to a pull request that touches
   Windows or macOS code to run those jobs too.
2. **Finish the branch** (`github-release finish`): the changelog is updated with git-cliff and a
   pull request `release: <x.y>` into `main` is opened. It runs the full matrix (Ubuntu with X11 and
   headless sway, Windows, macOS) and `cargo deny`.
3. **Merge it with a merge commit** (never squash). The push to `main` starts `release.yml`.
4. **`release.yml`** determines the version with GitVersion (`GitVersion.yml`), builds the CLI
   binaries (Linux x86_64 and arm64, macOS arm64, Windows x86_64) and the installers (`nsis`, `dmg`), and publishes the GitHub release `v<version>` with the notes of git-cliff. Until the first
   stable version the version is `0.1.0-beta.<n>` and the release is marked as a pre-release. The
   release branch is deleted after the merge.

To try the workflow without releasing, run it by hand (`gh workflow run release.yml`) or change the
file in a pull request: both build everything with the version `0.0.<run number>` and publish nothing.

## What a release contains

- `daimos` (the CLI) as an archive per platform with a SHA-256 checksum.
- The installers of the app: an `.exe` installer on Windows and a `.dmg` on macOS. **Linux has no
  installer yet**: the shared installer job (`GravionLabs/ci`, `tauri/installers`) installs the Tauri
  libraries but not `libgtk-layer-shell-dev`, which the Wayland overlay of Daimos needs to build, so the
  Linux build fails there. Once that package can be added (an input for extra Linux packages in the
  shared workflow), add the Linux entry back to `installer-matrix` in `release.yml`. The `.deb`
  also has to declare `libgtk-layer-shell0` as a dependency (`bundle.linux.deb.depends`, already set).
  Until then Linux users take the CLI archive and build the app from source (README). There is no
  `rpm` (the bundler hangs on the runner), no AppImage and no MSI (it needs a numeric version) yet.
- **The packages are not signed.** Windows SmartScreen and macOS Gatekeeper warn about them; on
  macOS, open the app with a right click on first start. Signed builds and the auto-update are the
  work of the "Signed builds and packages" feature before 1.0.

## Before the first release

The manual gate of the foundation epic (#1) has to be done by someone with the machines: the hotkey
on KDE Plasma and GNOME with Wayland, the panel over a full-screen app on macOS, the window on
Windows at 150 % scaling. See `docs/platform/` for what each platform does.
