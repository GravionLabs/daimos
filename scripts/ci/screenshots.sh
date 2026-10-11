#!/usr/bin/env bash
# Starts the real app in the current X11 display (run it under xvfb-run) once per color scheme and
# saves a screenshot of the palette with a few fixture apps: target/screenshots/<scheme>.png.
# The scheme is forced through the settings file, the apps are fixture .desktop files, so the
# pictures do not depend on what the machine has installed. Needs ImageMagick (`import`).
set -euo pipefail

app="${APP:-target/debug/daimos-app}"
cli="${CLI:-target/debug/daimos}"
out="${OUT:-target/screenshots}"
mkdir -p "$out"

# WebKitGTK without a GPU, on X11 even when a Wayland session is around.
export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
export GDK_BACKEND=x11
unset WAYLAND_DISPLAY

work="$(mktemp -d)"
trap 'kill "${app_pid:-}" 2>/dev/null || true; rm -rf "$work"' EXIT
export XDG_CONFIG_HOME="$work/config" XDG_DATA_HOME="$work/data" XDG_CACHE_HOME="$work/cache"
mkdir -p "$XDG_DATA_HOME/applications"
for entry in "Calculator|Calculator|Perform arithmetic" "Files|File Manager|Browse files" "Firefox|Web Browser|Browse the web" "Terminal|Terminal Emulator|Run commands"; do
  IFS='|' read -r name generic comment <<<"$entry"
  cat >"$XDG_DATA_HOME/applications/$name.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=$name
GenericName=$generic
Comment=$comment
Exec=true
DESKTOP
done

for scheme in light dark; do
  mkdir -p "$XDG_CONFIG_HOME/dev.gravionlabs.daimos"
  printf '{"theme":"%s"}\n' "$scheme" >"$XDG_CONFIG_HOME/dev.gravionlabs.daimos/settings.json"
  log="$work/$scheme.log"
  "$app" >"$log" 2>&1 &
  app_pid=$!
  # Start-up can take half a minute on a runner without a session bus.
  for _ in $(seq 1 450); do grep -q "listening on" "$log" && break; sleep 0.2; done
  grep -q "listening on" "$log" || { cat "$log" >&2; echo "the app did not start" >&2; exit 1; }
  echo "--- start-up log ($scheme)"; cat "$log"
  # The web view needs a moment to load: events sent before that are lost.
  sleep 3
  # The first scan runs in the background; wait until the fixture apps are indexed.
  for _ in $(seq 1 100); do "$cli" index status 2>/dev/null | grep -q "^[1-9]" && break; sleep 0.2; done
  "$cli" show "fi"
  sleep 3
  import -window root "$out/$scheme.png"
  "$cli" quit
  wait "$app_pid" || true
  echo "saved $out/$scheme.png"
done
