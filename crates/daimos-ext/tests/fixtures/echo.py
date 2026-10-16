"""A script extension for the integration tests of the one-shot protocol (standard library only).

The text of a query picks the behavior; the action of an `action` request does the same.
Every start is recorded in `runs.log` and the process id in `pid.txt`, both in the working
directory, so a test can see that (and whether) the process ran and whether it is gone.
"""

import json
import os
import sys
import threading
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


def conformance(request):
    """The same answer in both modes (query texts starting with `conf`): the one-shot and the
    long-running run of the script must give identical items."""
    text = request["text"]
    items(request, "conf one " + text, "conf two " + text)
    items(request, "conf three " + text, done=True)


def query(request):
    text = request["text"]
    if text.startswith("conf"):
        return conformance(request)
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


def serve():
    """Loop mode (`echo.py loop`): answer every line until EOF, several requests at once.

    Records `start` in `starts.log`, `<id> <type>` per request in `requests.log` and
    `<id> <cancels>` per cancel in `cancels.log`. Query texts: `slow` answers after 0.5 s unless
    cancelled, `stubborn` answers after 0.5 s even when cancelled, `partial` sends one batch at
    once and the rest after 0.5 s unless cancelled, `pid` answers with the process id, `die` exits
    with code 4, `garbage` prints a line that is not JSON, `hang` never answers.
    """
    with open("starts.log", "a") as starts:
        starts.write("start\n")
    with open("pid.txt", "w") as pid:
        pid.write(str(os.getpid()))
    cancelled = set()
    lock = threading.Lock()

    def record(name, line):
        with lock:
            with open(name, "a") as log:
                log.write(line + "\n")

    def answer(request, honor_cancel):
        """Waits up to 0.5 s; False when the request was cancelled meanwhile."""
        for _ in range(50):
            if honor_cancel and request["id"] in cancelled:
                return False
            time.sleep(0.01)
        return not (honor_cancel and request["id"] in cancelled)

    def reply(request):
        text = request.get("text", "")
        if text.startswith("conf"):
            return conformance(request)
        if request["type"] == "action":
            if request["action"] == "star":
                send({"id": request["id"], "type": "result", "toast": "Starred " + request["item"], "view": None})
            return
        if text == "slow" or text == "stubborn":
            if answer(request, text == "slow"):
                items(request, text + " done", done=True)
        elif text == "partial":
            items(request, "partial first")
            if answer(request, True):
                items(request, "partial last", done=True)
        elif text == "pid":
            items(request, str(os.getpid()), done=True)
        elif text == "hang":
            time.sleep(60)
        else:
            items(request, "echo " + text, done=True)

    for line in sys.stdin:
        if not line.strip():
            continue
        request = json.loads(line)
        record("requests.log", "%s %s" % (request["id"], request["type"]))
        if request["type"] == "cancel":
            cancelled.add(request["cancels"])
            record("cancels.log", "%s %s" % (request["id"], request["cancels"]))
        elif request.get("text") == "die":
            sys.stderr.write("dying\n")
            sys.exit(4)
        elif request.get("text") == "garbage":
            print("this is not json", flush=True)
        else:
            threading.Thread(target=reply, args=(request,), daemon=True).start()


if len(sys.argv) > 1 and sys.argv[1] == "loop":
    serve()
else:
    main()
