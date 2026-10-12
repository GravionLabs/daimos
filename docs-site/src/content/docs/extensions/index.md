---
title: Extensions
description: Write extensions for Daimos as scripts or WebAssembly components.
banner:
  content: This section is not complete yet. The extension host is still in development.
---

Extensions add commands to Daimos. An extension is a script (any language, JSON Lines over
standard input and output) or a WebAssembly component.

The tutorial, the manifest and protocol reference, and the WIT reference land with the extension
host. Until then, the design is in
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

## Pages

- [Manifest reference](manifest/): the sections and fields of `extension.toml`.
