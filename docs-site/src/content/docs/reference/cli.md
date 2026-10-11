---
title: Command line
description: Control the running Daimos with the daimos command.
---

`daimos` talks to the running application.

| Command                     | Effect                                                        |
| --------------------------- | ------------------------------------------------------------- |
| `daimos toggle`             | Show the window if it is hidden, hide it if it is shown.      |
| `daimos show [text]`        | Show the window, optionally with text in the search field.    |
| `daimos hide`               | Hide the window.                                              |
| `daimos quit`               | Quit the running Daimos.                                      |
| `daimos index …`            | Work with the application index.                              |

Run `daimos --help` for the options of each command. `daimos toggle` is how you bind Daimos to a key
on compositors without the GlobalShortcuts portal, see [Wayland setup](../../guides/wayland/).
