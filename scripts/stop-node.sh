#!/usr/bin/env bash
#
# Stop the Weft node started by run-node.sh (xray + frpc + control plane). Tears down the
# persistent services so the node stops carrying traffic and won't restart on reboot.
#   ./scripts/stop-node.sh           # stop (config kept; re-run run-node.sh to start again)
#   ./scripts/stop-node.sh --purge   # also remove ~/.weft + the service definitions
set -euo pipefail

PURGE="${1:-}"
SK="$HOME/.weft"
OS=$(uname -s)

echo "→ stopping Weft node…"
if [ "$OS" = "Linux" ]; then
  for u in weft-node-cp weft-node-frpc weft-node-xray; do
    sudo systemctl disable --now "$u" 2>/dev/null || true
    if [ "$PURGE" = "--purge" ]; then sudo rm -f "/etc/systemd/system/$u.service"; fi
  done
  sudo systemctl daemon-reload 2>/dev/null || true
else
  LA="$HOME/Library/LaunchAgents"
  for l in xray frpc cp; do
    launchctl unload -w "$LA/com.weft.node.$l.plist" 2>/dev/null || true
    if [ "$PURGE" = "--purge" ]; then rm -f "$LA/com.weft.node.$l.plist"; fi
  done
fi

if [ "$PURGE" = "--purge" ]; then
  rm -rf "$SK"
  echo "✅ node stopped + purged (~/.weft and services removed)."
else
  echo "✅ node stopped. Your keys/config remain in ~/.weft — run ./scripts/run-node.sh to start again."
  echo "   (Your on-chain registration stays; the node is just offline until restarted.)"
fi
