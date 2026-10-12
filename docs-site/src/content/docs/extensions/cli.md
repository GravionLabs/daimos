---
title: Extension commands
description: List the installed extensions and their state with the daimos command.
since: '0.4'
banner:
  content: This page is not complete yet. More extension commands are coming with the extension host.
---

`daimos ext` works with the extensions that the running Daimos has loaded. Like every `daimos`
command it talks to the running application, so start Daimos first.

## List the extensions

```sh
daimos ext list
```

The output has one line per extension:

```text
NAME          VERSION  RUNTIME  STATE
broken        -        -        error
  extension.toml:2: extension.name: `My Ext` is not a safe name: use lowercase letters, digits, `-` and `_` (at most 64)
github-repos  0.2.0    wasm     enabled
```

| Column    | Meaning                                                                       |
| --------- | ----------------------------------------------------------------------------- |
| `NAME`    | The extension's name, or its folder name when the manifest cannot be read     |
| `VERSION` | The `version` from the manifest, `-` when the manifest cannot be read         |
| `RUNTIME` | `script` or `wasm`, `-` when the manifest cannot be read                      |
| `STATE`   | `enabled`, or `error` with the message on the next line                       |

An extension in the `error` state is disabled until you fix its `extension.toml`. Daimos reloads it
when you save the file, see [hot reload](../#hot-reload). When no extension is installed the
command prints `no extensions installed`.

If Daimos is not running, the command prints a message and exits with code 2.

## Platform notes

The command is the same on X11, Wayland (KDE and GNOME), Windows, and macOS. The extension folder
is different on each, see [where extensions live](../#where-extensions-live).
