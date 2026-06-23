#!/usr/bin/env bash
# Launch a REAL multi-process Weft network (separate weft-node processes over TCP) +
# expose a SOCKS5 proxy that tunnels through them. Each node writes a bootstrap manifest;
# relays seed forwarding from the shared manifest dir; the client connects to the same dir.
#
# Usage: scripts/weft-vpn-net.sh [num_relays=2] [socks_port=1080]
set -euo pipefail
RELAYS="${1:-2}"
SOCKS_PORT="${2:-1080}"
DIR="${WEFT_NET_DIR:-.weft-net}"
NODE=./target/release/weft-node
VPN=./target/release/weft-vpn
rm -rf "$DIR"; mkdir -p "$DIR/peers"
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT INT TERM

# Auto-assigned ports (/tcp/0) — each node's real bound address lands in its manifest, so
# there are never port conflicts. Capabilities default to WIREGUARD|RELAY|EXIT; the last
# selected hop acts as the exit. Start RELAYS relay nodes + 1 dedicated exit node.
for i in $(seq 1 "$RELAYS"); do
  WEFT_NODE_ID="$i" WEFT_NODE_GEO=$((100 + i)) \
    WEFT_NODE_LISTEN="/ip4/127.0.0.1/tcp/0" \
    WEFT_MANIFEST="$DIR/peers/node-$i.json" WEFT_PEERS="$DIR/peers" \
    "$NODE" > "$DIR/node-$i.log" 2>&1 &
  PIDS+=($!)
done
WEFT_NODE_ID=100 WEFT_NODE_GEO=200 \
  WEFT_NODE_LISTEN="/ip4/127.0.0.1/tcp/0" \
  WEFT_MANIFEST="$DIR/peers/exit.json" WEFT_PEERS="$DIR/peers" \
  "$NODE" > "$DIR/exit.log" 2>&1 &
PIDS+=($!)

echo "[net] started $((RELAYS+1)) real weft-node processes; waiting for manifests + routes…"
for i in $(seq 1 40); do
  n=$(find "$DIR/peers" -name '*.json' 2>/dev/null | wc -l | tr -d ' ')
  [ "$n" -ge $((RELAYS + 1)) ] && break; sleep 0.5
done
sleep 3   # let each relay seed peer routes (2s bootstrap delay inside the node)
echo "[net] $(find "$DIR/peers" -name '*.json' | wc -l | tr -d ' ') node manifests ready in $DIR/peers"

# Run the VPN client against the REAL external nodes.
WEFT_HOPS="$((RELAYS + 1))" WEFT_PEERS="$DIR/peers" \
  "$VPN" socks "127.0.0.1:$SOCKS_PORT" > "$DIR/vpn.log" 2>&1 &
PIDS+=($!)
sleep 3
echo "[net] SOCKS5 proxy on 127.0.0.1:$SOCKS_PORT (over $((RELAYS+1)) real nodes). Logs in $DIR/."
echo "[net] test: curl --proxy socks5h://127.0.0.1:$SOCKS_PORT https://example.com"
echo "[net] Ctrl-C to stop everything."
wait
