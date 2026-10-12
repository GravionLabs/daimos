---
title: Extensions
description: Write extensions for Daimos as scripts or WebAssembly components.
banner:
  content: This section is not complete yet. The extension host is still in development.
---

Extensions add commands to Daimos. An extension is a script (any language, JSON Lines over
standard input and output) or a WebAssembly component.

The tutorial and the WIT reference land with the extension host. Until then, the design is in
[`docs/specs/extensions.md`](https://github.com/GravionLabs/daimos/blob/main/docs/specs/extensions.md).

## Where extensions live

Each extension is a folder with an `extension.toml` and the files it needs. Daimos looks for these
folders in your configuration directory:

| Platform                             | Extension directory                                |
| ------------------------------------ | -------------------------------------------------- |
| Linux (X11, Wayland on KDE or GNOME) | `~/.config/daimos/extensions`                      |
| Windows                              | `%APPDATA%\daimos\extensions`                      |
| macOS                                | `~/Library/Application Support/daimos/extensions`  |

The layout is the same everywhere:

```text
extensions/
  github-repos/
    extension.toml
    icon.png
    main.wasm
  notes/
    extension.toml
    main.py
```

Folders without an `extension.toml`, files, and folders whose name starts with a dot are ignored.
A broken extension does not stop the others from loading. Two extensions cannot share a name: the
second one (in folder-name order) is reported as an error.

## Script extensions run unsandboxed

A script extension is a program that runs with your rights. It can do anything you can do, so
Daimos says so wherever its results appear: each result of a script extension carries the badge
`Runs unsandboxed` in the list and in the header of the detail pane, and the extension has the same
label in the settings. Wasm extensions are sandboxed and can only use the capabilities they
declare; they show no badge on results, and `Sandboxed` in the settings.

Before an extension runs for the first time, and again when an update asks for more, you confirm its
capabilities, see [confirmation and updates](manifest/#confirmation-and-updates) and
[`daimos ext install`](cli/#install-an-extension).

## Hot reload

Daimos watches the extension directory. When you save a file in an extension folder, add a folder,
or delete one, that extension is read again about a quarter of a second after the last change. You
do not restart Daimos. A new keyword works at once.

## A broken manifest

If an `extension.toml` cannot be loaded, Daimos disables only that extension and keeps running.
You see one result with the title "`<folder>` could not be loaded" and the message under it. The
message names the file, the line, and the field, for example:

```text
extension.toml:3: extension.name: `My Ext` is not a safe name: use lowercase letters, digits, `-` and `_` (at most 64)
```

The result shows in the palette when the search is empty or when you type part of the folder
name. Each broken extension has exactly one result, and the other extensions keep answering. Fix
the file and save it: the error disappears and the extension works again without a restart. `daimos ext
list` shows the state of every extension, see the [manifest reference](manifest/) for the rules.

## Pages

- [Manifest reference](manifest/): the sections and fields of `extension.toml`.
- [Script extensions](scripts/): write an extension as a script that speaks JSON Lines.
- [Extension commands](cli/): `daimos ext install`, `confirm` and `list`.
