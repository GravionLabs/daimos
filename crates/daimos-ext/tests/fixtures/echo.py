"""A script extension for the integration tests of the one-shot protocol (standard library only).

The text of a query picks the behavior; the action of an `action` request does the same.
Every start is recorded in `runs.log` and the process id in `pid.txt`, both in the working
directory, so a test can see that (and whether) the process ran and whether it is gone.
"""

import json
import os
import sys
import time


def send(message):
    print(json.dumps(message), flush=True)


def items(request, *titles, done=False):
    send(
        {
            "id": request["id"],
            "type": "items",
            "items": [
                {
                    "id": title,
                    "title": title,
                    "subtitle": "from echo.py",
                    "actions": [
                        {"id": "open", "title": "Open", "kind": "open", "url": "https://example.com/" + title},
                        {"id": "star", "title": "Star", "kind": "custom"},
                    ],
                }
                for title in titles
            ],
            "done": done,
        }
    )


def query(request):
    text = request["text"]
    if text == "exit1":
        items(request, "before-exit")
        sys.stderr.write("warming up\n")
        sys.stderr.write("fixture failed\n")
        sys.exit(1)
    if text == "hang":
        items(request, "before-hang")
        time.sleep(60)
    elif text == "malformed":
        items(request, "before-garbage")
        print("this is not json", flush=True)
        items(request, "after-garbage")
    elif text == "log":
        sys.stderr.write("log line one\n")
        sys.stderr.write("log line two\n")
        items(request, "logged", done=True)
    elif text == "tail":
        items(request, "tail", done=True)
        time.sleep(60)  # done must end the query before this
    else:
        items(request, "echo " + text, "again " + text)
        items(request, "last " + text, done=True)


def action(request):
    name = request["action"]
    if name == "star":
        send({"id": request["id"], "type": "result", "toast": "Starred " + request["item"], "view": None})
    elif name == "quiet":
        send({"id": request["id"], "type": "result", "toast": None, "view": None})
    elif name == "fail":
        sys.stderr.write("star failed\n")
        sys.exit(3)
    elif name == "hang":
        time.sleep(60)


def main():
    request = json.loads(sys.stdin.readline())
    with open("runs.log", "a") as runs:
        runs.write(request["type"] + "\n")
    with open("pid.txt", "w") as pid:
        pid.write(str(os.getpid()))
    if request["type"] == "query":
        query(request)
    elif request["type"] == "action":
        action(request)


main()
