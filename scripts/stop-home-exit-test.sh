#!/usr/bin/env bash
set -euo pipefail

WORK="${WEFT_TEST_DIR:-$HOME/.weft-home-exit-test}"

if command -v launchctl >/dev/null 2>&1; then
  UID_VALUE="$(id -u)"
  launchctl bootout "gui/${UID_VALUE}" "$WORK/frpc.plist" >/dev/null 2>&1 || true
  launchctl bootout "gui/${UID_VALUE}" "$WORK/xray.plist" >/dev/null 2>&1 || true
fi

stop_pid() {
  local file="$1"
  if [ -f "$file" ]; then
    local pid
    pid="$(cat "$file" 2>/dev/null || true)"
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      sleep 1
      kill -9 "$pid" 2>/dev/null || true
    fi
    rm -f "$file"
  fi
}

stop_pid "$WORK/frpc.pid"
stop_pid "$WORK/xray.pid"

echo "home-exit test stopped"
