#!/usr/bin/env bash
#
# Print a connection link (vless://…) you can paste into V2Box, Happ, sing-box or
# Hiddify to connect through Weft. Run:
#
#     ./scripts/share-link.sh
#
# It starts a gateway and prints the link. For a phone on the same Wi-Fi, replace
# the host in the link with your computer's local IP. Pass a custom address with
# ./scripts/share-link.sh 0.0.0.0:9000
#
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

ADDR="${1:-0.0.0.0:8443}"
echo "→ Building…"
cargo build -p weft-vpn --release -q
echo "→ Starting the gateway at ${ADDR}. Your connection link:"
exec ./target/release/weft-vpn vless "${ADDR}" --tls
