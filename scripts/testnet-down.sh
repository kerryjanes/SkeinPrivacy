#!/usr/bin/env bash
# Tear down the local Weft testnet started by testnet-up.sh.
set -euo pipefail
RUN_DIR="${WEFT_RUN_DIR:-.testnet}"
[ -f "$RUN_DIR/nodes.pid" ] && while read -r pid; do kill "$pid" 2>/dev/null || true; done < "$RUN_DIR/nodes.pid"
[ -f "$RUN_DIR/validator.pid" ] && kill "$(cat "$RUN_DIR/validator.pid")" 2>/dev/null || true
rm -rf "$RUN_DIR"
echo "[testnet] torn down."
