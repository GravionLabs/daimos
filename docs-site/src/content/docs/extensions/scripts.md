---
title: Script extensions
description: Write an extension as a script that talks JSON Lines over standard input and output.
since: '0.4'
banner:
  content: WebAssembly extensions are still in development.
---

A script extension is a program in any language. Daimos starts it, sends it a request as one line
of JSON, and reads the results from its output. You need nothing but a language that can read a
line and print JSON.

A script runs in one of two modes. In the one-shot mode, which is `long_running = false` and the
default, Daimos starts the program again for every query and every custom action. In the
[long-running mode](#long-running-mode) the program starts once and answers many requests. The
design is in
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

A `cancel` request exists for long-running scripts, see below. A one-shot program never receives
it, because Daimos stops the process instead.

## Long-running mode

Set `long_running = true` when starting the program costs too much for every keystroke, for example
because it loads a large index or opens a connection.

```toml
[runtime.script]
command = ["python3", "main.py"]
long_running = true
timeout_ms = 5000
idle_timeout_ms = 600000
```

- **Start.** Daimos starts the program the first time you use one of its commands and keeps it
  running. Queries are not delayed by the 150 ms wait of one-shot scripts.
- **Requests.** The program reads one request per line from standard input until the input ends,
  and answers on standard output. The `id` of the requests increases by one for every request.
  Every answer repeats the `id` of its request, so you can answer in any order, and you can work
  on several requests at the same time. A query ends with a message that has `"done": true`; an
  `action` ends with its `result`. Standard input ends when Daimos stops the program, so exit when
  it does.
- **Cancel.** When you type again, the earlier query is superseded and Daimos sends

  ```json
  {"id": 7, "type": "cancel", "cancels": 5}
  ```

  `cancels` is the `id` of the request to drop. Stop working on it if you can. You send no answer
  to a `cancel`, and anything you still send for request 5 is ignored.
- **Time limit.** `timeout_ms` applies to every request. When one takes longer, Daimos kills the
  program, shows `<name> took too long`, and starts it again with the next request.
- **Crashes.** When the program exits or closes its output on its own, requests that were waiting
  fail with the last line it wrote to standard error. The next request starts it again after a
  short wait: 250 ms, then 500 ms, then 1 s. Daimos restarts the program at most three times
  within any 60 seconds. After that the extension shows one item, `<name> keeps crashing`, with
  the last line of standard error, until the extension is reloaded, which
  happens when you save a file in its folder (see [hot reload](../#hot-reload)). Stops that Daimos asked for do not count.
- **Idle.** After `idle_timeout_ms` without a request (default `600000`, ten minutes) Daimos
  closes the standard input of the program and stops it if it does not exit within half a second.
  The next query starts it again. `idle_timeout_ms` has no effect with `long_running = false`.
- **Unload and quit.** The program is stopped when the extension is unloaded, reloaded or removed,
  and when Daimos quits.

A Python program that answers every query and honors `cancel`:

```python
import json
import sys

cancelled = set()
for line in sys.stdin:
    request = json.loads(line)
    if request["type"] == "cancel":
        cancelled.add(request["cancels"])
    elif request["type"] == "query" and request["id"] not in cancelled:
        print(json.dumps({
            "id": request["id"],
            "type": "items",
            "done": True,
            "items": [{"id": "echo", "title": "You typed: " + request["text"]}],
        }), flush=True)
```

Both modes return the same items for the same program. Write to standard error for diagnostics in
both; the log file is the same.

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
| A long-running program keeps crashing | `<name> keeps crashing`     | The last non-empty line it wrote to standard error       |

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
- **"keeps crashing".** Read the log, fix the program, then save a file in its folder to reload it.
- Other problems are collected in the [extension commands](../cli/) page, which shows whether
  Daimos loaded the extension at all.
