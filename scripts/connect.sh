#!/usr/bin/env bash
#
# Use Weft from your computer as a local proxy. Run:
#
#     ./scripts/connect.sh
#
# Then point your browser or apps at the SOCKS5 address it prints (default
# 127.0.0.1:1080). Optionally pass a port: ./scripts/connect.sh 1090
#
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

PORT="${1:-1080}"
echo "→ Building…"
cargo build -p weft-vpn --release -q
echo "→ Weft is running at 127.0.0.1:${PORT} (SOCKS5). Point your apps there. Ctrl-C to stop."
exec ./target/release/weft-vpn socks "127.0.0.1:${PORT}"
