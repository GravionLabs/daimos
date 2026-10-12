"""A script extension for the integration tests of the script protocol (standard library only).

One-shot mode answers the one request it is started with. With the argument `loop` it answers
every request until its input ends and honors `cancel`; `runs.log` then gets a line `loop` per
start and `requests.log` every line received.

The text of a query picks the behavior; the action of an `action` request does the same.
Every start is recorded in `runs.log` and the process id in `pid.txt`, both in the working
directory, so a test can see that (and whether) the process ran and whether it is gone.
"""

import json
import os
import sys
import threading
import time

SEND_LOCK = threading.Lock()
CANCELLED = set()


def send(message):
    with SEND_LOCK:
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
    elif text == "id":
        items(request, "id %d" % request["id"], done=True)
    elif text in ("slow", "late"):
        # Answers from a thread after 300 ms while the loop keeps reading. "slow" honors a
        # cancel for it, "late" answers anyway.
        def answer():
            time.sleep(0.3)
            if text == "slow" and request["id"] in CANCELLED:
                return
            items(request, text + " answer", done=True)

        threading.Thread(target=answer, daemon=True).start()
    elif text == "crash":
        sys.stderr.write("crashing on request\n")
        sys.exit(2)
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


def serve():
    """Loop mode (`echo.py loop`): answer every line until the input ends."""
    with open("runs.log", "a") as runs:
        runs.write("loop\n")
    with open("pid.txt", "w") as pid:
        pid.write(str(os.getpid()))
    for line in sys.stdin:
        if not line.strip():
            continue
        request = json.loads(line)
        with open("requests.log", "a") as log:
            log.write(line)
        if request["type"] == "cancel":
            CANCELLED.add(request["cancels"])
        elif request["type"] == "query":
            query(request)
        elif request["type"] == "action":
            action(request)


def main():
    if sys.argv[1:] == ["loop"]:
        serve()
        return
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
