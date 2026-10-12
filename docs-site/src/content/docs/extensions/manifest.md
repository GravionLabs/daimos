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

Only the table that matches `runtime` is used. How a script extension talks to Daimos is
described in [script extensions](../scripts/).

| Table              | Field          | Meaning                                                   |
| ------------------ | -------------- | --------------------------------------------------------- |
| `[runtime.script]` | `command`      | The program and its arguments, run in the extension folder |
| `[runtime.script]` | `long_running` | Keep the process alive between queries (default `false`)  |
| `[runtime.script]` | `timeout_ms`   | Milliseconds a query or action may take before the process is killed (default `5000`, must be greater than 0) |
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

### Confirmation and updates

An extension does not run until you confirm its capabilities. Daimos stores what you confirmed, with
the version, in `.consent.json` inside the extension folder, outside the manifest. Until then the
extension is listed as `needs-confirmation`, runs nothing, and shows no results.

When a new version of the manifest asks for more than you confirmed, the extension is disabled again
until you confirm the new set. These count as more: a new network host, a new path, a path with a
wider mode (`:rw` where you confirmed `:ro`; a path without a suffix counts as `:ro`), wider
clipboard access, and a new keyring entry. A new version that only removes capabilities keeps
running, and so does an edit that changes nothing in `[capabilities]`.

### In the settings

The **Extensions** section of the settings lists every extension with its version, its state, and
a label for its runtime: `Runs unsandboxed` for a script extension, `Sandboxed` for a wasm
extension. Under each one you see the capabilities it declares, one per line (files with read only
or read and write, network hosts, clipboard access, keyring entries).

An extension that needs a confirmation shows a **Confirm** button. After an update, the
capabilities that are new compared with what you confirmed are marked `(new)`. Nothing runs until
you press the button; it does the same as `daimos ext confirm`, see
[extension commands](../cli/#confirm-an-update).

## `[[preferences]]`

Settings you fill in for the extension.

| Field      | Required | Meaning                                                              |
| ---------- | -------- | -------------------------------------------------------------------- |
| `name`     | yes      | Identifier of the preference                                         |
| `title`    | yes      | Label shown to you                                                   |
| `type`     | yes      | `text`, `password` (kept in the OS keyring), `number`, or `checkbox` |
| `required` | no       | The extension cannot run until it is set (default `false`)           |

### Where the values are stored

You set the values in the settings. `text`, `number`, and `checkbox` values are stored in Daimos's
settings database. `password` values never go there: they are stored in the OS keyring, with the
service `daimos` and the user `<extension name>/<preference name>` (for example
`github-repos/token`), and read from it each time a query runs. They do not appear in logs or in
`daimos ext list`.

- **Linux** needs a running Secret Service, such as GNOME Keyring or KWallet. Without one, a
  required password preference makes the extension answer with a message that the OS keyring is not
  available; start the service and set the value again.
- **Windows** uses the Credential Manager and **macOS** uses the Keychain.

A required preference without a value stops the extension from running: its result names the
missing preference.

The settings page shows the preferences of each extension as a form under its capabilities:
text and number fields, a checkbox, and for a password a masked field that stays empty and only
says `Set` or `Not set`. Required preferences are marked `(required)`. Type a new password and
press **Save** to replace it, or **Clear** to remove it.

## How commands and keywords route queries

Each command is a source of results with the id `<extension name>.<command name>`, for example
`github-repos.search`.

- A query whose first word is one of the command's `keywords`, followed by a space, goes to that
  command only. `gh daimos` sends `daimos` to every command that lists `gh`. Keywords ignore case.
- Any other query goes to all commands and to the built-in sources, and the results are ranked
  together.
- A command without keywords is only reached by the second case.
- Every result from an extension carries the extension's `title` as a label, so you can see where
  it came from.

One-shot script extensions answer queries, see [script extensions](../scripts/). Wasm extensions
and long-running scripts are still in development, so their commands return no results yet.

## Validation

Daimos checks the manifest when it loads the extension. An extension with a problem is disabled and
the message names the file, the line, and the field:

```text
extension.toml:9: commands[0].name: `Search/All` is not a safe name: use lowercase letters, digits, `-` and `_`
```

The rules:

- `extension.name`, every command `name`, and every preference `name` are identifiers: lowercase
  letters, digits, `-` and `_`, starting with a letter or digit, at most 64 characters.
- `extension.title` and `extension.version` are not empty; so are command and preference titles.
- `runtime` is `script` or `wasm`, and the matching `[runtime.script]` or `[runtime.wasm]` table
  exists. The table of the other runtime is an error.
- `[runtime.script]` has a `command` that names a program, and its `timeout_ms` is greater than 0; `[runtime.wasm]` has a `component`.
- There is at least one `[[commands]]` entry, and command names are unique.
- A command `mode` is `view` or `no-view`.
- Keywords are not empty and contain no spaces.
- Preference names are unique, and `type` is one of the types above.
- Unknown fields are errors.

More examples of messages:

```text
extension.toml:6: extension.runtime: runtime is "wasm" but there is no [runtime.wasm] table
extension.toml:18: commands[1].name: command `search` is defined twice
extension.toml:11: commands[0].mode: unknown variant `popup`, expected `view` or `no-view`
extension.toml:2: extension.title: missing field `title`
```

## Platform notes

The manifest is the same on X11, Wayland (KDE and GNOME), Windows, and macOS. Paths in `command`
and `component` are relative to the extension folder.
