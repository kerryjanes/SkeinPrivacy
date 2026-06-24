#!/usr/bin/env bash
#
# Run a Weft node in one command. It creates your operator key (once), builds the
# node, registers it on-chain, and starts relaying traffic. Just run:
#
#     ./scripts/run-node.sh
#
# Re-run any time to start your node again. Everything is stored under ~/.weft.
#
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

DATA="${WEFT_HOME:-$HOME/.weft}"
KEY="$DATA/operator.key"
MANIFEST="$DATA/node.json"
mkdir -p "$DATA"

# Sensible defaults — no API keys needed. Override only if you know you need to.
export WEFT_CLUSTER="${WEFT_CLUSTER:-devnet}"
export WEFT_RPC_URL="${WEFT_RPC_URL:-https://api.devnet.solana.com}"

# 1. Operator key — created once, reused forever. This single key is both your
#    node's identity and your on-chain wallet.
if [ ! -f "$KEY" ]; then
  echo "→ Creating your operator key (one time)…"
  openssl rand -hex 96 > "$KEY"
  chmod 600 "$KEY"
fi
export WEFT_OPERATOR_KEY="$(cat "$KEY")"
export WEFT_MANIFEST="$MANIFEST"

# 2. Build (first run takes a few minutes; instant afterwards).
echo "→ Building the node…"
cargo build -p weft-node --release -q

# 3. Start the node; it publishes its manifest, then keeps relaying.
echo "→ Starting your node…"
./target/release/weft-node &
NODE_PID=$!
trap 'kill $NODE_PID 2>/dev/null || true' EXIT
for _ in $(seq 1 30); do [ -f "$MANIFEST" ] && break; sleep 1; done

# 4. Register on-chain (idempotent; auto-funds on devnet).
echo "→ Registering your node on-chain…"
if pnpm --filter @weft/registry-provision node:register --manifest "$MANIFEST" --key "$KEY"; then
  echo "→ Registered. Your node is live and earning."
else
  echo "→ Couldn't register automatically (see above). Your node is still running and relaying."
fi

echo "→ Node running. Press Ctrl-C to stop."
wait $NODE_PID
