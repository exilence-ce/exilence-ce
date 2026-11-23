#!/usr/bin/env bash
# Starts a bundled build (assets embedded, CSP applied, unlike a devUrl run)
# on a fresh data directory and waits for the UI's "UI mounted" log line.
# Fails on an error line from the webview (ui/index.html forwards CSP
# violations and page errors), on an early exit or after 60 s.
#
#   (cd ui && trunk build --release)
#   cargo build -p exilence-ce-ng --features tauri/custom-protocol
#   tools/ui-smoke.sh target/debug/exilence-ce-ng
#
# Needs a display and a session bus (CI: dbus-run-session -- xvfb-run -a) and
# no running Exilence CE NG (the single-instance plugin would hand over to it).
set -euo pipefail

bin=$1
data=$(mktemp -d)
log="$data/com.exilence.exilence-ce-ng/logs/exilence-ce-ng.log"
XDG_DATA_HOME=$data "$bin" &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; rm -rf "$data"' EXIT

for _ in $(seq 120); do
  if grep -q ' ui: UI mounted' "$log" 2>/dev/null; then
    echo "UI mounted"
    exit 0
  fi
  if grep ' ERROR ui: ' "$log" 2>/dev/null; then
    exit 1
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "the app exited before the UI mounted"
    cat "$log" 2>/dev/null || true
    exit 1
  fi
  sleep 0.5
done
echo "the UI did not mount within 60 s"
cat "$log" 2>/dev/null || true
exit 1
