#!/usr/bin/env bash
# Stand up a local Weft testnet on one host: a fresh validator, all programs deployed,
# genesis run, and N weft-node relay daemons joined into a Kademlia DHT mesh. This is
# the developer one-shot behind the M8 "incentivized testnet" milestone; the heavy data
# plane is also covered socket-free by the cell-transport integration tests.
#
# Usage: ./scripts/testnet-up.sh [num_nodes=5]
# Tear down with ./scripts/testnet-down.sh
set -euo pipefail

NODES="${1:-5}"
RUN_DIR="${WEFT_RUN_DIR:-.testnet}"
LEDGER="$RUN_DIR/ledger"
mkdir -p "$RUN_DIR"

echo "[testnet] starting validator…"
solana-test-validator --reset --quiet --ledger "$LEDGER" &
echo $! > "$RUN_DIR/validator.pid"
sleep 8

solana config set --url http://127.0.0.1:8899 >/dev/null
solana airdrop 100 >/dev/null 2>&1 || true

echo "[testnet] building + deploying programs…"
anchor build
anchor deploy --provider.cluster localnet

echo "[testnet] running genesis (mint + distribution)…"
WEFT_CLUSTER=localnet WEFT_RPC_URL=http://127.0.0.1:8899 \
  pnpm --filter @weft/genesis start

echo "[testnet] spawning $NODES relay daemons…"
BOOT=""
for i in $(seq 1 "$NODES"); do
  port=$((9100 + i))
  WEFT_NODE_ID="$i" WEFT_NODE_GEO=$((100 + i)) \
    WEFT_NODE_LISTEN="/ip4/127.0.0.1/tcp/$port" \
    cargo run --quiet --release -p weft-node > "$RUN_DIR/node-$i.log" 2>&1 &
  echo $! >> "$RUN_DIR/nodes.pid"
  [ -z "$BOOT" ] && BOOT="/ip4/127.0.0.1/tcp/$port"
  sleep 1
done

echo "[testnet] up: $NODES nodes; validator pid $(cat "$RUN_DIR/validator.pid")."
echo "[testnet] logs in $RUN_DIR/node-*.log; tear down with scripts/testnet-down.sh"
