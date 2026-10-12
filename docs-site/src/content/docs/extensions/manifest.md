---
title: Manifest reference
description: The sections and fields of extension.toml.
since: '0.4'
banner:
  content: This page is not complete yet. The extension host is still in development.
---

Every extension has an `extension.toml` in its directory. It names the extension, lists its
commands, says how to run it, and declares what it may touch. The design is in
[`docs/specs/extensions.md`](https://github.com/GravionLabs/daimos/blob/main/docs/specs/extensions.md).

```toml
[extension]
name = "github-repos"
title = "GitHub Repos"
version = "0.2.0"
description = "Search and open your GitHub repositories"
author = "you"
runtime = "wasm"

[[commands]]
name = "search"
title = "Search Repos"
mode = "view"
keywords = ["gh", "repo"]

[runtime.wasm]
component = "main.wasm"

[capabilities]
network = ["api.github.com"]
clipboard = "write"

[[preferences]]
name = "token"
title = "GitHub token"
type = "password"
required = true
```

Unknown tables and fields are errors, so a typo does not silently do nothing.

## `[extension]`

| Field         | Required | Meaning                                              |
| ------------- | -------- | ---------------------------------------------------- |
| `name`        | yes      | Identifier of the extension, also its directory name |
| `title`       | yes      | Name shown to you                                    |
| `version`     | yes      | Version of the extension                             |
| `runtime`     | yes      | `script` or `wasm`                                   |
| `description` | no       | One line about what it does                          |
| `author`      | no       | Who wrote it                                         |

## `[[commands]]`

One entry per command.

| Field      | Required | Meaning                                                                   |
| ---------- | -------- | ------------------------------------------------------------------------- |
| `name`     | yes      | Identifier of the command, unique within the extension                    |
| `title`    | yes      | Name shown to you                                                         |
| `mode`     | no       | `view` (shows a list, the default) or `no-view` (runs and exits)          |
| `keywords` | no       | Words that route a query to this command, for example `["gh", "repo"]`    |

## `[runtime.script]` and `[runtime.wasm]`

Only the table that matches `runtime` is used.

| Table              | Field          | Meaning                                                   |
| ------------------ | -------------- | --------------------------------------------------------- |
| `[runtime.script]` | `command`      | The program and its arguments, run in the extension folder |
| `[runtime.script]` | `long_running` | Keep the process alive between queries (default `false`)  |
| `[runtime.wasm]`   | `component`    | Path of the WebAssembly component                         |

## `[capabilities]`

What the extension may use. Daimos enforces it for wasm extensions and only displays it for script
extensions, which run with your rights.

| Field        | Meaning                                                          |
| ------------ | ---------------------------------------------------------------- |
| `network`    | Host names the extension may reach                               |
| `filesystem` | Paths it may open, with `:ro` for read only                      |
| `clipboard`  | `read`, `write`, or `read-write`                                 |
| `keyring`    | Names of secrets it may read from the OS keyring                 |

## `[[preferences]]`

Settings you fill in for the extension.

| Field      | Required | Meaning                                                              |
| ---------- | -------- | -------------------------------------------------------------------- |
| `name`     | yes      | Identifier of the preference                                         |
| `title`    | yes      | Label shown to you                                                   |
| `type`     | yes      | `text`, `password` (kept in the OS keyring), `number`, or `checkbox` |
| `required` | no       | The extension cannot run until it is set (default `false`)           |

## Platform notes

The manifest is the same on X11, Wayland (KDE and GNOME), Windows, and macOS. Paths in `command`
and `component` are relative to the extension folder.
