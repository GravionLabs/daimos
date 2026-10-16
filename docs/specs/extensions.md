# Extensions

Two runtimes, one data model, one manifest. A script extension is a program in any language that
speaks JSON Lines over stdin and stdout. A wasm extension is a WebAssembly component that
implements the daimos's WIT interface and runs inside wasmtime with only the capabilities its
manifest declares.

| Runtime | Best for                                     | Isolation                                  | Call latency          |
| ------- | -------------------------------------------- | ------------------------------------------ | --------------------- |
| Script  | Quick personal commands, wrapping CLIs       | None: runs with the user's rights, labeled | ~10–30 ms warm        |
| Wasm    | Shared and catalog extensions, anything fast | Sandboxed: declared capabilities only      | < 5 ms in-process     |

## Layout

```
~/.config/daimos/extensions/<name>/
  extension.toml      manifest
  icon.png
  main.py             script runtime: any executable, or
  main.wasm           wasm runtime: a component
```

The CLI scaffolds both: `daimos ext new --script python`, `daimos ext new --wasm rust`.
`daimos ext dev <path>` loads an extension from a working directory with hot reload.

## Manifest

```toml
[extension]
name = "github-repos"
title = "GitHub Repos"
version = "0.2.0"
description = "Search and open your GitHub repositories"
author = "you"
runtime = "wasm"            # or "script"

[[commands]]
name = "search"
title = "Search Repos"
mode = "view"               # view: shows a list; no-view: runs and exits
keywords = ["gh", "repo"]

[runtime.script]            # only for runtime = "script"
command = ["python3", "main.py"]
long_running = true         # keep the process alive between queries

[runtime.wasm]              # only for runtime = "wasm"
component = "main.wasm"

[capabilities]              # enforced for wasm, displayed for script
network = ["api.github.com"]
filesystem = ["$HOME/.config/gh:ro"]
clipboard = "write"
keyring = ["github-token"]

[[preferences]]
name = "token"
title = "GitHub token"
type = "password"           # stored in the OS keyring
required = true
```

On install, the user sees the capabilities as a list and confirms. A wasm extension that asks for a
network host it did not declare gets an error, not a prompt. A script extension shows a permanent
"runs unsandboxed" label.

## Script protocol

JSON Lines, UTF-8, one message per line, requests from the daimos on stdin and responses on
stdout. stderr goes to the extension log. Every request carries an `id`; the response repeats it.

```jsonc
// daimos → extension
{"id": 1, "type": "query", "command": "search", "text": "lau", "preferences": {"token": "…"}}
// extension → daimos (may be sent several times for the same id to stream)
{"id": 1, "type": "items", "items": [{"id": "r1", "title": "daimos", "subtitle": "gravionlabs", "actions": [{"id": "open", "title": "Open in browser", "kind": "open", "url": "https://…"}]}], "done": false}
{"id": 1, "type": "items", "items": [], "done": true}
// daimos → extension when an action is not a built-in kind
{"id": 2, "type": "action", "command": "search", "item": "r1", "action": "star"}
// extension → daimos
{"id": 2, "type": "result", "toast": "Starred", "view": null}
// daimos → extension
{"id": 3, "type": "cancel", "cancels": 1}
```

- **One-shot mode** (`long_running = false`): the daimos starts the process per query, writes one
  request, reads until `done: true` or exit. Simplest to write; costs a process spawn per
  keystroke, so results are debounced to 150 ms.
- **Long-running mode**: the process starts at first use and stays alive while the extension is
  loaded. Queries are not debounced. Requests get increasing ids from one counter; several can be
  in flight and the answers are routed by id, so they may interleave. Cancels let it drop stale
  work, and late answers for a cancelled id are ignored. Custom actions use the same process. The
  daimos restarts it on crash with a backoff (250 ms, doubling) and gives up after three restarts
  within a minute until the extension is reloaded; it stops the process after `idle_timeout_ms`
  without a request, and on unload, reload and exit.
- Built-in action kinds (`open`, `copy`, `paste`, `run`) are executed by the daimos, so a script
  never has to touch the clipboard or the shell itself.

## Wasm runtime

The host is wasmtime with the component model. The interface is a WIT package; language bindings
come from `wit-bindgen` (Rust, C), `componentize-js` (JavaScript, TypeScript), `componentize-py`
(Python), and TinyGo.

```wit
package daimos:ext@0.1.0;

interface types {
  record item { id: string, title: string, subtitle: option<string>, icon: option<icon>,
                keywords: list<string>, actions: list<action>, detail: option<detail> }
  record action { id: string, title: string, kind: action-kind, payload: option<string>,
                  shortcut: option<string> }
  variant action-kind { open, copy, paste, run, push, custom }
  variant icon { named(string), file(string), data(string) }
  record detail { markdown: option<string>, metadata: list<tuple<string, string>> }
  variant view { items(list<item>), detail(detail), form(form), none }
  record form { fields: list<field>, submit: string }
  record field { name: string, title: string, kind: string, required: bool }
}

interface host {
  use types.{item};
  log: func(level: string, message: string);
  preference: func(name: string) -> option<string>;
  cache-get: func(key: string) -> option<list<u8>>;
  cache-set: func(key: string, value: list<u8>, ttl-seconds: u64);
  toast: func(message: string);
}

world extension {
  import host;
  import wasi:http/outgoing-handler@0.2.0;   // only if the manifest declares network hosts
  export query: func(command: string, text: string) -> list<types.item>;
  export action: func(command: string, item: string, action: string) -> types.view;
}
```

- **Capabilities map to WASI**: `filesystem` entries become preopened directories; `network`
  entries become an allow-list in the host's `wasi:http` implementation; `keyring` entries are read
  through `host.preference` and never reach the guest as files.
- **Instances are pooled**: components are compiled once and cached on disk (wasmtime's
  serialization), instances are reused between queries, and a query runs with an epoch deadline so
  a runaway extension is interrupted, not the daimos.
- **Streaming** is not in the first interface version: a wasm query returns its list in one call,
  which is fine inside the 5 ms budget. A later version may add a `stream` export.

## Conformance

`examples/` holds the same extension in four languages: Bash and Python as scripts, Rust and
JavaScript as components. The conformance suite runs each one against a fixed input and expects
identical item lists. This is the gate of the M2 milestone.

## Catalog (M4)

A Git repository with one directory per extension: manifest, icon, and a signed `main.wasm` or the
script sources. The daimos clones it, shows the entries, and installs with the capability prompt.
Only wasm extensions are listed as "sandboxed"; script extensions in the catalog require review.
