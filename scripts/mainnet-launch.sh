#!/usr/bin/env bash
# Weft mainnet launch — run at the launch window once the pump.fun mint (CA) is known.
#
#   ./scripts/mainnet-launch.sh <MINT_CA> [--yes]
#
# Given the reward-token mint, this brings the on-chain protocol live in two steps:
#   1. core:init   -> creates registry + staking config + distributor + reward vault,
#                     auto-creates the treasury ATA (owned by the admin), wires poster
#                     and dispute authorities. reward_mint = the CA.
#   2. nft:provision -> creates the node-cNFT collection + Bubblegum tree and points the
#                       registry at them.
# Both steps are idempotent: re-running after a partial launch resumes cleanly.
#
# PREREQUISITE: the weft program must already be DEPLOYED on mainnet under the current
# declare_id! (the original 6riawCPV was closed; a rebrand relaunch deploys a fresh id first —
# see LAUNCH.md). This script does NOT deploy or upgrade the program; it only initializes state.
# The reward mint (CA) may be classic SPL or Token-2022 — initCore detects the mint's owner
# program at runtime and threads it through. Non-recoverable spend here is ~0.23 SOL — mostly
# the cNFT merkle-tree rent (~0.22) + tx fees. The frontend reads the mint from the distributor
# at runtime, so no rebuild is needed after this runs — only the control-plane needs its
# WEFT_MINT set to the CA (see scripts/mainnet-cutover.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

MINT="${1:-${WEFT_MINT:-}}"
YES=0
for a in "$@"; do [[ "$a" == "--yes" || "$a" == "-y" ]] && YES=1; done
[[ "${WEFT_LAUNCH_YES:-0}" == "1" ]] && YES=1

if [[ -z "${MINT}" || "${MINT}" == "--yes" || "${MINT}" == "-y" ]]; then
  echo "usage: $0 <MINT_CA> [--yes]" >&2
  exit 1
fi

# --- fixed launch parameters (public keys — safe to commit) ---
export WEFT_CLUSTER="${WEFT_CLUSTER:-mainnet-beta}"
export WEFT_KEYPAIR="${WEFT_KEYPAIR:-${HOME}/.config/solana/weft-admin.json}"
export WEFT_MINT="${MINT}"
# Aggregator poster key living at /etc/weft/authority.json on the relay VPS.
export WEFT_POSTER_AUTHORITY="${WEFT_POSTER_AUTHORITY:-DEg6vvwNmkhaV9aTUaEUbhCG5AbFKNvGq8egiqScF1nq}"
# Dispute + treasury owner default to the admin wallet 9AY1on6.
export WEFT_DISPUTE_AUTHORITY="${WEFT_DISPUTE_AUTHORITY:-9AY1on6okCYep7uKVTmJQbGHa7uqv5sQ4AwfuJhEkqJw}"

# --- RPC: prefer an explicit override, then the local (uncommitted) Helius key,
#     then public mainnet-beta as a last resort. ---
if [[ -z "${WEFT_RPC_URL:-}" ]]; then
  HELIUS_CFG="${HOME}/Documents/helius/config.json"
  if [[ -f "${HELIUS_CFG}" ]]; then
    KEY="$(node -e "process.stdout.write((require('${HELIUS_CFG}').apiKey)||'')" 2>/dev/null || true)"
    if [[ -n "${KEY}" ]]; then
      export WEFT_RPC_URL="https://mainnet.helius-rpc.com/?api-key=${KEY}"
    fi
  fi
fi
export WEFT_RPC_URL="${WEFT_RPC_URL:-https://api.mainnet-beta.solana.com}"

ADMIN="$(solana address -k "${WEFT_KEYPAIR}")"
BAL="$(solana balance "${ADMIN}" --url "${WEFT_RPC_URL}" | awk '{print $1}')"
RPC_MASKED="$(printf '%s' "${WEFT_RPC_URL}" | sed -E 's/(api-key=)[^&]+/\1***/')"

cat <<EOF

======================= WEFT MAINNET LAUNCH =======================
  cluster           : ${WEFT_CLUSTER}
  rpc               : ${RPC_MASKED}
  admin (fee payer) : ${ADMIN}   balance ${BAL} SOL
  reward mint (CA)  : ${WEFT_MINT}
  treasury owner    : ${WEFT_TREASURY_OWNER:-${ADMIN}}
  poster authority  : ${WEFT_POSTER_AUTHORITY}
  dispute authority : ${WEFT_DISPUTE_AUTHORITY}

  This will: create treasury ATA -> initialize_core -> provision cNFT collection+tree.
  Program is already deployed; this only initializes state. Spend ~0.23 SOL (mostly
  the cNFT merkle-tree rent, non-recoverable).
====================================================================
EOF

if [[ "${YES}" != "1" ]]; then
  read -r -p "Type LAUNCH to proceed: " CONFIRM
  [[ "${CONFIRM}" == "LAUNCH" ]] || { echo "aborted."; exit 1; }
fi

echo ""; echo "[launch] step 1/2 — core:init"
pnpm --filter @weft/registry-provision core:init

echo ""; echo "[launch] step 2/2 — nft:provision"
pnpm --filter @weft/registry-provision nft:provision

echo ""
cat <<EOF
==================== LAUNCH ON-CHAIN COMPLETE ======================
  Manifest: services/registry-provision/manifests/${WEFT_CLUSTER}.json

  Next (backend + verify):
    1. ./scripts/mainnet-cutover.sh ${WEFT_MINT}      # point relay VPS at mainnet + CA, restart
    2. Fund the node-payout wallet once buyback \$WEFT lands in the treasury (admin ATA).
       Node rewards are paid directly from DEg6vvwNmkhaV9aTUaEUbhCG5AbFKNvGq8egiqScF1nq:
         spl-token transfer ${WEFT_MINT} <amount> DEg6vvwNmkhaV9aTUaEUbhCG5AbFKNvGq8egiqScF1nq \\
           --fund-recipient --url "\$WEFT_RPC_URL" --owner "\$WEFT_KEYPAIR"
       (It can't overpay: transfers are capped at its balance, so an underfunded wallet
        just pauses withdrawals — never goes negative.)
    3. Verify: curl https://vpn.weftnetwork.net:8089/price  (mint=${WEFT_MINT}, faucet=false)
       and load https://weftnetwork.net/app — distributor now resolves the token.
====================================================================
EOF
