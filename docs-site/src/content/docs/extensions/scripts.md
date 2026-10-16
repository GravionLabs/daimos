---
title: Script extensions
description: Write an extension as a script that talks JSON Lines over standard input and output.
since: '0.4'
banner:
  content: This page covers script extensions. WebAssembly extensions are still in development.
---

A script extension is a program in any language. Daimos starts it, sends it a request as one line
of JSON, and reads the results from its output. You need nothing but a language that can read a
line and print JSON.

A script runs in one of two modes. In one-shot mode, `long_running = false` and the default,
Daimos starts the program again for every query and every custom action. In
[long-running mode](#long-running-mode) the program starts once and answers many requests. Most of
this page applies to both; the differences are in their own section. The design is in
[`docs/specs/extensions.md`](https://github.com/GravionLabs/daimos/blob/main/docs/specs/extensions.md).

## The manifest

```toml
[extension]
name = "greeter"
title = "Greeter"
version = "0.1.0"
runtime = "script"

[[commands]]
name = "greet"
title = "Greet"
keywords = ["hi"]

[runtime.script]
command = ["python3", "main.py"]
timeout_ms = 5000
```

`command` is the program and its arguments. The program starts in the extension folder, so
`main.py` is found next to `extension.toml`. A program with a path such as `bin/run` is relative to
the folder too. `timeout_ms` is how long one query or action may take before Daimos kills the
process; it defaults to `5000` and must be greater than 0. See the
[manifest reference](../manifest/) for every field.

## What happens on a query

This is the one-shot mode; a long-running program skips the wait and keeps running, see
[long-running mode](#long-running-mode).

1. You type. Daimos waits 150 ms after the last keystroke, so typing fast does not start a
   process per letter. A query that you replace during that wait never starts anything.
2. Daimos starts the program, writes one `query` request on its standard input, and closes the
   input.
3. The program writes `items` messages on its standard output, one per line. Daimos shows the
   items as they arrive, so a slow program can send the first results early.
4. The query ends when a message has `"done": true`, when the program exits, or when the time is up.
   Daimos then stops the process if it is still running.

If you type again while a process is running, Daimos stops that process and starts the next query.

## Messages

Every message is one line of UTF-8 JSON. Blank lines are ignored, and so are fields Daimos does not
know, so an extension written today keeps working when new fields are added. Every request has an
`id`, and the answer repeats it.

### Request: query

```json
{"id": 1, "type": "query", "command": "greet", "text": "lau", "preferences": {}}
```

`command` is the name from `[[commands]]`. `text` is what you typed, without the keyword.
`preferences` is empty for now; values from `[[preferences]]` are not passed to scripts yet.

### Response: items

```json
{"id": 1, "type": "items", "items": [{"id": "r1", "title": "daimos", "subtitle": "gravionlabs", "actions": [{"id": "open", "title": "Open in browser", "kind": "open", "url": "https://github.com/GravionLabs/daimos"}]}], "done": false}
{"id": 1, "type": "items", "items": [], "done": true}
```

Send as many `items` messages as you like for one request. The last one can carry `"done": true`;
a program that exits without it works too. An item has these fields:

| Field      | Required | Meaning                                                                     |
| ---------- | -------- | --------------------------------------------------------------------------- |
| `id`       | yes      | Identifier of the item, used when you run one of its actions                |
| `title`    | yes      | The main line                                                               |
| `subtitle` | no       | A second line                                                               |
| `icon`     | no       | A themed icon name, a `data:` URL, or the path of an image file             |
| `keywords` | no       | Extra words the search matches                                              |
| `actions`  | no       | What you can do with the item; the first one runs on **Enter**              |
| `detail`   | no       | `{"markdown": "..."}` or `{"metadata": [["Key", "Value"]]}`                 |

Every item carries the extension's title as a label, so you see where it came from.

### Actions

An action has an `id`, a `title`, a `kind`, and optionally a `payload` and a `shortcut` such as
`Ctrl+K`.

| Kind     | What Daimos does                                                              | `payload`              |
| -------- | ----------------------------------------------------------------------------- | ---------------------- |
| `open`   | Opens it with the default application                                         | URL or path (`url` also works) |
| `copy`   | Copies it to the clipboard                                                    | The text               |
| `paste`  | Copies it and pastes it into the application you came from                    | The text               |
| `run`    | Starts a command line without a shell; quotes group words                     | The command line       |
| `custom` | Sends the action back to your program                                         | none                   |

The four built-in kinds are done by Daimos itself. Your program is not started for them, and it
never has to touch the clipboard or a shell. Opening uses `xdg-open` on Linux, `open` on macOS and
the default handler on Windows.

A `custom` action starts the program with an `action` request:

```json
{"id": 2, "type": "action", "command": "greet", "item": "r1", "action": "star"}
```

Answer with one `result` message:

```json
{"id": 2, "type": "result", "toast": "Starred", "view": null}
```

`toast` is a short message that Daimos shows in the window; use `null` for none. `view` is
reserved for later and is ignored. A program that exits without a `result` counts as done, without
a message. The same time limit applies, and a failure shows as an error message.

A `cancel` request exists for long-running scripts, see
[long-running mode](#long-running-mode). A one-shot program never receives it, because Daimos
stops the process instead.

## Long-running mode

A program that is slow to start, or that keeps a connection or a cache, can stay alive between
queries:

```toml
[runtime.script]
command = ["python3", "main.py"]
long_running = true
timeout_ms = 5000
idle_timeout_ms = 600000
```

**Start.** Daimos starts the program on the first query or custom action of the extension, not
when it loads the extension. There is no 150 ms wait: every keystroke is sent at once, because no
process has to start for it.

**Requests and ids.** The program reads requests from standard input, one per line, until the
input closes. Custom actions arrive on the same input as queries. Every request has a new `id`,
counting up from 1 for as long as the extension is loaded, also across restarts. Several requests
can be open at the same time, and the answers may come in any order: Daimos hands every message to
the request whose `id` it repeats, and ignores messages for an `id` it no longer waits for.

**Cancel.** When you type again before a query is done, Daimos sends a `cancel` request and stops
waiting for the old query:

```json
{"id": 4, "type": "cancel", "cancels": 3}
```

`cancels` is the `id` of the request to drop; the cancel has an `id` of its own. Stop that work if
you can. Anything you still send for it is ignored, so you do not have to answer a cancel.

**Time limit.** `timeout_ms` applies to every request. When the time is up, the query shows
`<name> took too long` and Daimos kills the process; the next request starts a new one.

**Crashes.** A program that exits on its own, or writes a line that is not valid JSON, has
crashed. The open requests show `<name> failed` with the last line from standard error, and the
next request starts the program again after a short wait that doubles with every restart: 250 ms,
then 500 ms, then 1 s. After three restarts within a minute, the next crash stops the extension:
every query shows `<name> keeps crashing` with the last line from standard error, and nothing is
started any more. Editing a file of the extension (which reloads it) or restarting Daimos gives it
a fresh start. A process that Daimos stops itself, at the time limit, after the idle time, or on a
reload, never counts as a crash.

**Idle timeout.** When no request has been open for `idle_timeout_ms` (default ten minutes),
Daimos closes the standard input of the program, so a loop over the input ends, and kills the
program if it is still running half a second later. The next query starts it again. A request
that takes longer than the idle time keeps the process alive.

**Reload and exit.** When you edit, disable or remove the extension, Daimos stops its process; the
next request starts the new version. When Daimos exits, it stops every long-running process.

A long-running version of the greeter in Python:

```python
import json
import sys

for line in sys.stdin:
    request = json.loads(line)
    if request["type"] == "query":
        print(json.dumps({
            "id": request["id"],
            "type": "items",
            "done": True,
            "items": [{"id": "echo", "title": "You typed: " + request["text"]}],
        }), flush=True)
    elif request["type"] == "action":
        print(json.dumps({"id": request["id"], "type": "result", "toast": "Done", "view": None}),
              flush=True)
    # "cancel": nothing to stop, every answer above is immediate.
```

The log works as in one-shot mode; a long-running program appends to it for as long as it runs.

## Errors, stderr and the log

Write diagnostics to standard error. Daimos appends it, line by line, to a log file named after
the extension:

| Platform                             | Log file                                                                  |
| ------------------------------------ | ------------------------------------------------------------------------- |
| Linux (X11, Wayland on KDE or GNOME) | `~/.local/share/dev.gravionlabs.daimos/logs/<name>.log`                   |
| Windows                              | `%APPDATA%\dev.gravionlabs.daimos\logs\<name>.log`                        |
| macOS                                | `~/Library/Application Support/dev.gravionlabs.daimos/logs/<name>.log`    |

A log above 1 MiB is moved to `<name>.log.1` before the next run, replacing the older copy. If the
log cannot be written, the query still works.

When something goes wrong, you see one error item below the items that already arrived:

| What happened                       | Title                         | Second line                                              |
| ----------------------------------- | ----------------------------- | -------------------------------------------------------- |
| The program exits with a non-zero code | `<name> failed`            | The last non-empty line it wrote to standard error, or `exited with code N` |
| The time is up                      | `<name> took too long`        | `timed out after 5 s`                                    |
| The program cannot be started       | `<name> failed`               | `cannot start <program>: ...`                            |
| A line is not valid JSON            | `<name> failed`               | The line, shortened to 200 characters                    |
| A long-running program crashed too often | `<name> keeps crashing`  | The last non-empty line it wrote to standard error, or `restarted too often, stopped` |

Exit with code 0 when everything is fine, and with another code after printing the reason to
standard error, for example `echo "token expired" >&2; exit 1`.

## Examples

A Python script that answers every query with the text you typed and an action to copy it:

```python
import json
import sys

request = json.loads(sys.stdin.readline())
print(json.dumps({
    "id": request["id"],
    "type": "items",
    "done": True,
    "items": [{
        "id": "echo",
        "title": "You typed: " + request["text"],
        "actions": [{"id": "copy", "title": "Copy", "kind": "copy", "payload": request["text"]}],
    }],
}), flush=True)
```

Use `flush=True`, or the output waits in a buffer until the program exits. With
`command = ["python3", "main.py"]` this is the whole extension.

The same in Bash, using [`jq`](https://jqlang.org/) to read and write JSON:

```bash
#!/usr/bin/env bash
read -r request
printf '%s' "$request" | jq -c '{
  id, type: "items", done: true,
  items: [{id: "echo", title: ("You typed: " + .text),
           actions: [{id: "copy", title: "Copy", kind: "copy", payload: .text}]}]
}'
```

Set `command = ["bash", "main.sh"]`.

## Platform notes

The protocol is the same on X11, Wayland (KDE and GNOME), Windows, and macOS.

- On Windows the program starts without a console window, so nothing flashes on screen. Use the
  name of the interpreter that your Windows has, usually `python` instead of `python3`. Bash
  scripts need a Bash on the path, such as the one from Git for Windows.
- On Linux and macOS, `python3` is the usual name. A script that is itself executable can be named
  directly with a relative path, for example `command = ["./main.sh"]`.
- Script extensions run with your rights and are not sandboxed. Daimos shows the capabilities
  from the manifest, but does not enforce them for scripts.

## Troubleshooting

- **Nothing appears.** Open the log file of the extension, run the program by hand and type the
  request line, and check that every line of output is valid JSON.
- **Results come late.** The program writes to a buffer. Flush after every line.
- **"took too long".** Raise `timeout_ms`, or send a first `items` message early and the rest
  later.
- Other problems are collected in the [extension commands](../cli/) page, which shows whether
  Daimos loaded the extension at all.
