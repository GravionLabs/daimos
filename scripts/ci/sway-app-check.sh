#!/usr/bin/env bash
# Runs the real app inside a headless sway session (use scripts/ci/run-in-sway.sh) and drives it
# with the CLI: the window must be a layer-shell surface, and `daimos toggle` twice must show and
# then hide it. The app logs every visibility change; that log is what is asserted, because sway
# does not list layer surfaces in `get_tree`.
set -euo pipefail

app="${APP:-target/debug/daimos-app}"
cli="${CLI:-target/debug/daimos}"
log="$(mktemp)"
trap 'kill "${app_pid:-}" 2>/dev/null || true; rm -f "$log"' EXIT

# WebKitGTK without a GPU.
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
export GDK_BACKEND=wayland

"$app" >"$log" 2>&1 &
app_pid=$!

wait_for() { # <pattern> <count>
  for _ in $(seq 1 100); do
    [ "$(grep -c "$1" "$log" || true)" -ge "$2" ] && return 0
    kill -0 "$app_pid" 2>/dev/null || { echo "the app exited:" >&2; cat "$log" >&2; return 1; }
    sleep 0.2
  done
  echo "timed out waiting for '$1' x$2; log:" >&2
  cat "$log" >&2
  return 1
}

wait_for "listening on" 1
wait_for "window mode: LayerShell" 1

"$cli" toggle
wait_for "window visibility: shown" 1
"$cli" toggle
wait_for "window visibility: hidden" 1
"$cli" show "hello"
wait_for "window visibility: shown" 2
"$cli" hide
wait_for "window visibility: hidden" 2

# A second run of the CLI after quit must report "not running" with exit code 2.
"$cli" quit
wait "$app_pid" || true
status=0
"$cli" toggle || status=$?
[ "$status" -eq 2 ] || { echo "expected exit code 2 after quit, got $status" >&2; exit 1; }
echo "sway app check passed"
