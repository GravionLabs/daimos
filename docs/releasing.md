# Releasing

A release is the merge of a release branch into `main`. Everything after the merge is automatic.

## Steps

1. **Cut the branch** from `main`: `release/<x.y>` (the `github-release` skill's `cut`). All work for
   the release is squash-merged into it through pull requests; pull requests into a release branch
   run the two Linux jobs of `ci.yml`. Add the `full-matrix` label to a pull request that touches
   Windows or macOS code to run those jobs too.
2. **Finish the branch** (`github-release finish`): the changelog is updated with git-cliff and a
   pull request `release: <x.y>` into `main` is opened. It runs the full matrix (Ubuntu with headless
   sway, Windows, macOS) and `cargo deny`. The X11 job is not part of it; add the `x11` label to a
   pull request to run it.
3. **Merge it with a merge commit** (never squash). The push to `main` starts `release.yml`. Stable
   versions are numbered from the last stable tag: the next merge is a patch, a commit message with
   `+semver: minor` (`+semver: major`) bumps that part, so a minor release carries it in one commit.
   While the last tag is a pre-release (before 0.3.0) GitVersion ignores it; `next-version` in
   `GitVersion.yml` fixed the number of 0.2 and 0.3 and is deleted after `v0.3.0`.
4. **`release.yml`** determines the version with GitVersion (`GitVersion.yml`), builds the CLI
   binaries (Linux x86_64 and arm64, macOS arm64, Windows x86_64) and the installers (`deb`, `nsis`,
   `dmg`), and publishes the GitHub release `v<version>` with the notes of git-cliff. From 0.3 on the
   version is stable (`0.3.0`); the earlier ones were `-beta.<n>` pre-releases. The
   release branch is deleted after the merge.

To try the workflow without releasing, run it by hand (`gh workflow run release.yml`) or change the
file in a pull request: both build everything with the version `0.0.<run number>` and publish nothing.

## What a release contains

- `daimos` (the CLI) as an archive per platform with a SHA-256 checksum.
- The installers of the app: a `.deb` on Linux (it depends on `libgtk-layer-shell0`), an `.exe`
  installer on Windows and a `.dmg` on macOS. There is no `rpm` (the bundler hangs on the runner), no
  AppImage and no MSI (it needs a numeric version) yet. The Linux build installs
  `libgtk-layer-shell-dev` through `installer-linux-packages` of the shared workflow.
- **The packages are not signed.** Windows SmartScreen and macOS Gatekeeper warn about them; on
  macOS, open the app with a right click on first start. Signed builds and the auto-update are the
  work of the "Signed builds and packages" feature before 1.0.

## Before the first release

The manual gate of the foundation epic (#1) has to be done by someone with the machines: the hotkey
on KDE Plasma and GNOME with Wayland, the panel over a full-screen app on macOS, the window on
Windows at 150 % scaling. See `docs/platform/` for what each platform does.
