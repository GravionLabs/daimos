#!/usr/bin/env bash
# Runs a command inside a headless sway session with the XDG desktop portals running:
#   scripts/ci/run-in-sway.sh cargo test --workspace
# Used by the Ubuntu Wayland job of ci.yml; locally it needs sway, xdg-desktop-portal and
# xdg-desktop-portal-wlr installed.
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 64
fi

runtime_dir="$(mktemp -d)"
chmod 700 "$runtime_dir"
export XDG_RUNTIME_DIR="$runtime_dir"
export XDG_SESSION_TYPE=wayland
export XDG_CURRENT_DESKTOP=sway
export WLR_BACKENDS=headless
export WLR_LIBINPUT_NO_DEVICES=1
export WLR_RENDERER=pixman
# Only xdg-desktop-portal-wlr runs here, which has no GlobalShortcuts interface (see the
# portal_unsupported test of daimos-platform).
export DAIMOS_EXPECT_NO_PORTAL=1
unset DISPLAY

cleanup() {
  kill "${sway_pid:-}" "${portal_pid:-}" "${portal_wlr_pid:-}" 2>/dev/null || true
  sleep 0.5
  rm -rf "$runtime_dir" 2>/dev/null || true
}
trap cleanup EXIT

dbus-run-session -- bash -euo pipefail -c '
  sway --unsupported-gpu >"$XDG_RUNTIME_DIR/sway.log" 2>&1 &
  sway_pid=$!
  for _ in $(seq 1 50); do
    socket="$(ls "$XDG_RUNTIME_DIR" | grep -m1 "^wayland-[0-9]*$" || true)"
    [ -n "$socket" ] && break
    sleep 0.2
  done
  if [ -z "${socket:-}" ]; then cat "$XDG_RUNTIME_DIR/sway.log"; echo "sway did not start" >&2; exit 1; fi
  export WAYLAND_DISPLAY="$socket"
  export SWAYSOCK="$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*.sock | head -1)"
  /usr/libexec/xdg-desktop-portal-wlr >"$XDG_RUNTIME_DIR/portal-wlr.log" 2>&1 &
  sleep 1
  /usr/libexec/xdg-desktop-portal >"$XDG_RUNTIME_DIR/portal.log" 2>&1 &
  sleep 1
  status=0
  "$@" || status=$?
  kill $sway_pid 2>/dev/null || true
  exit $status
' bash "$@"
