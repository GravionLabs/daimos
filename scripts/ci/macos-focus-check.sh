#!/usr/bin/env bash
# macOS: showing the panel must not take the activation away from the frontmost app.
# TextEdit is the "previous app"; `lsappinfo` reads the frontmost app without any permission.
set -euo pipefail

app="${APP:-target/debug/daimos-app}"
cli="${CLI:-target/debug/daimos}"
log="$(mktemp)"
trap 'kill "${app_pid:-}" 2>/dev/null || true; osascript -e "tell application \"TextEdit\" to quit" 2>/dev/null || true; rm -f "$log"' EXIT

frontmost() { lsappinfo info -only name "$(lsappinfo front)" | sed -E 's/.*"LSDisplayName"="?([^"]*)"?.*/\1/'; }

open -a TextEdit
for _ in $(seq 1 50); do [ "$(frontmost)" = "TextEdit" ] && break; sleep 0.2; done
[ "$(frontmost)" = "TextEdit" ] || { echo "TextEdit did not become frontmost: $(frontmost)" >&2; exit 1; }

"$app" >"$log" 2>&1 &
app_pid=$!
for _ in $(seq 1 100); do grep -q "listening on" "$log" && break; sleep 0.2; done
grep -q "listening on" "$log" || { cat "$log" >&2; echo "the app did not start" >&2; exit 1; }

"$cli" show
for _ in $(seq 1 50); do grep -q "window visibility: shown" "$log" && break; sleep 0.2; done
grep -q "window visibility: shown" "$log" || { cat "$log" >&2; echo "the window was not shown" >&2; exit 1; }
sleep 1

now="$(frontmost)"
[ "$now" = "TextEdit" ] || { echo "the previous app lost the activation: frontmost is '$now'" >&2; cat "$log" >&2; exit 1; }

"$cli" hide
"$cli" quit
echo "macOS focus check passed"
