---
title: Install
description: Install Daimos on Linux, Windows, and macOS.
---

Download the installer for your platform from the
[latest release](https://github.com/GravionLabs/daimos/releases/latest). The packages are **not
signed yet**, so your system warns about them.

| Platform | File                         | Notes                                                                    |
| -------- | ---------------------------- | ------------------------------------------------------------------------ |
| Linux    | `Daimos_<version>_amd64.deb` | `sudo apt install ./Daimos_*_amd64.deb`; depends on `libgtk-layer-shell0`. |
| Windows  | `Daimos_<version>_x64-setup.exe` | SmartScreen warns: choose **More info → Run anyway**.                |
| macOS    | `Daimos_<version>_aarch64.dmg` | Open the app with a right click on the first start (Gatekeeper).       |

There is no rpm, AppImage or MSI yet.

## The command line

Each release also has an archive with the `daimos` command for every platform
(`daimos-<target>.tar.gz`, `.zip` on Windows) and a `SHA256SUMS` file. Put the binary on your
`PATH`; see the [command line reference](../../reference/cli/).

## First start

Press **Alt+Space** (the default hotkey) to open the window. On Wayland the first start asks the
desktop to confirm the shortcut; see [Wayland setup](../wayland/). The hotkey can be changed in the
settings (**Ctrl+,**).

## Build from source

See the [README](https://github.com/GravionLabs/daimos#build-and-run).
