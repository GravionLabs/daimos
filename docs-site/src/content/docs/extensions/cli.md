---
title: Extension commands
description: Install, confirm and list extensions with the daimos command.
since: '0.4'
banner:
  content: This page is not complete yet. More extension commands are coming with the extension host.
---

`daimos ext` installs extensions and shows the ones Daimos has loaded. `daimos ext list` talks to
the running application, so start Daimos first. `daimos ext install` and `daimos ext confirm` work
on the extension folder and do not need it; a running Daimos notices the change by itself.

## Install an extension

```sh
daimos ext install ./notes
daimos ext install https://github.com/example/daimos-notes.git
```

The source is a folder with an `extension.toml`, or a git URL that Daimos clones with `git`
(it needs `git` on your path). The extension is copied to the extension folder as
`<extension name>`; symlinks, the `.git` folder, and any `.consent.json` that comes with the
package are not copied. An extension that is already installed is not overwritten.

Before it is enabled you see what it asks for and are asked to confirm:

```text
notes 1.0.0 (script) by me
Runs unsandboxed: a script extension runs with your rights and can do anything you can.
It declares these capabilities:
  filesystem  $HOME/.config/gh (read only)
  network     api.example.com
  clipboard   write
  keyring     token
Enable this extension? [y/N]
```

Script extensions show `Runs unsandboxed`, wasm extensions show `Sandboxed`. Answer `y` to enable
the extension. Any other answer removes the copy again and the command exits with a non-zero code.

| Option        | Meaning                                                                    |
| ------------- | -------------------------------------------------------------------------- |
| `--yes`, `-y` | Confirm without asking, for scripts. Read the capabilities first.          |

Without `--yes`, a command that does not run in a terminal (for example in a pipe) cannot ask, so
it does not install the extension and exits with a non-zero code.

## Confirm an update

```sh
daimos ext confirm notes
```

When a new version of an extension asks for a capability you have not confirmed, it is disabled
and `daimos ext list` shows `needs-confirmation`. `daimos ext confirm <name>` shows the full list
with the new entries marked `(new)` and asks again. `--yes` skips the question. An extension that
is already confirmed says so and changes nothing. See
[confirmation and updates](../manifest/#confirmation-and-updates) for what counts as new.

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
| `STATE`   | `enabled`, `needs-confirmation`, or `error` with the message on the next line |

An extension in the `error` state is disabled until you fix its `extension.toml`. Daimos reloads it
when you save the file, see [hot reload](../#hot-reload). When no extension is installed the
command prints `no extensions installed`.

An extension in the `needs-confirmation` state runs nothing and the next line tells you to run
`daimos ext confirm <name>`.

If Daimos is not running, `daimos ext list` prints a message and exits with code 2.

## Platform notes

The commands are the same on X11, Wayland (KDE and GNOME), Windows, and macOS. The extension folder
is different on each, see [where extensions live](../#where-extensions-live). Cloning a git URL
needs `git` on your path on every platform.
