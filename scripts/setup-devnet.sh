#!/usr/bin/env bash
# Point the Solana CLI at devnet (or $WEFT_RPC_URL), ensure a funded keypair.
# Usage: ./scripts/setup-devnet.sh
set -euo pipefail

# Load .env if present (for WEFT_RPC_URL / HELIUS_API_KEY).
if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

RPC_URL="${WEFT_RPC_URL:-https://api.devnet.solana.com}"
echo "Using RPC: ${RPC_URL}"
solana config set --url "${RPC_URL}" >/dev/null

if ! solana account "$(solana address)" >/dev/null 2>&1; then
  echo "Funding keypair via airdrop..."
fi
solana airdrop 2 || echo "Airdrop failed (rate-limited?). Fund $(solana address) manually."
echo "Balance: $(solana balance)"
