#!/usr/bin/env bash
# Deploy the Weft program to an EXPLICIT cluster, echoing what it will do first. The program
# deploy is the single most expensive, irreversible mainnet action — never run a bare `anchor
# deploy` against an ambient cluster. This wrapper prints cluster + program id + admin balance and
# requires a typed confirmation before spending.
#
#   ./scripts/deploy-program.sh <devnet|mainnet-beta> [--yes]
#
# Uses the admin/upgrade authority at ~/.config/solana/weft-admin.json (override with WEFT_ADMIN_KEYPAIR).
# The Helius RPC key is read from the local ~/Documents/helius/config.json, never committed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLUSTER="${1:-}"
YES=0
for a in "$@"; do [[ "$a" == "--yes" || "$a" == "-y" ]] && YES=1; done

case "${CLUSTER}" in
  devnet|mainnet-beta) ;;
  *) echo "usage: $0 <devnet|mainnet-beta> [--yes]" >&2; exit 1 ;;
esac

ADMIN="${WEFT_ADMIN_KEYPAIR:-${HOME}/.config/solana/weft-admin.json}"
[[ -f "${ADMIN}" ]] || { echo "ERROR: admin keypair ${ADMIN} not found." >&2; exit 1; }

# Resolve the RPC (Helius local key for mainnet, else the public endpoint).
RPC="${WEFT_RPC_URL:-}"
if [[ -z "${RPC}" ]]; then
  if [[ "${CLUSTER}" == "mainnet-beta" ]]; then
    HELIUS_CFG="${HOME}/Documents/helius/config.json"
    if [[ -f "${HELIUS_CFG}" ]]; then
      KEY="$(node -e "process.stdout.write((require('${HELIUS_CFG}').apiKey)||'')" 2>/dev/null || true)"
      [[ -n "${KEY}" ]] && RPC="https://mainnet.helius-rpc.com/?api-key=${KEY}"
    fi
    RPC="${RPC:-https://api.mainnet-beta.solana.com}"
  else
    RPC="https://api.devnet.solana.com"
  fi
fi
RPC_MASKED="$(printf '%s' "${RPC}" | sed -E 's/(api-key=)[^&]+/\1***/')"

cd "${ROOT}"
DECLARE_ID="$(rg -o 'declare_id!\("[^"]+"\)' programs/weft/src/lib.rs | sed -E 's/.*"([^"]+)".*/\1/')"
ADMIN_ADDR="$(solana address -k "${ADMIN}")"
BAL="$(solana balance "${ADMIN_ADDR}" --url "${RPC}" 2>/dev/null || echo '?')"

cat <<EOF

===================== WEFT PROGRAM DEPLOY =====================
  cluster    : ${CLUSTER}
  rpc        : ${RPC_MASKED}
  program id : ${DECLARE_ID}   (declare_id! in programs/weft/src/lib.rs)
  admin      : ${ADMIN_ADDR}   (${BAL})
  artifact   : target/deploy/weft.so
================================================================
EOF

[[ -f target/deploy/weft.so ]] || { echo "ERROR: target/deploy/weft.so missing — run 'anchor build --ignore-keys' first." >&2; exit 1; }

# On mainnet, refuse to deploy at anything but the chosen mainnet program id — the fresh id must
# be set in declare_id!/Anchor.toml AND match the program keypair, or the program deploys at an
# address whose declare_id mismatches (DeclaredProgramIdMismatch on every instruction).
if [[ "${CLUSTER}" == "mainnet-beta" ]]; then
  : "${WEFT_MAINNET_PROGRAM_ID:?set WEFT_MAINNET_PROGRAM_ID to the fresh mainnet program id before a mainnet deploy}"
  [[ "${DECLARE_ID}" == "${WEFT_MAINNET_PROGRAM_ID}" ]] \
    || { echo "ERROR: declare_id ${DECLARE_ID} != WEFT_MAINNET_PROGRAM_ID ${WEFT_MAINNET_PROGRAM_ID}." >&2; exit 1; }
  KP_ID="$(solana address -k target/deploy/weft-keypair.json 2>/dev/null || true)"
  [[ "${KP_ID}" == "${DECLARE_ID}" ]] \
    || { echo "ERROR: program keypair ${KP_ID} != declare_id ${DECLARE_ID} — run 'anchor build --ignore-keys' first." >&2; exit 1; }
fi

if [[ "${YES}" != "1" ]]; then
  read -r -p "Type DEPLOY-${CLUSTER} to proceed: " CONFIRM
  [[ "${CONFIRM}" == "DEPLOY-${CLUSTER}" ]] || { echo "aborted."; exit 1; }
fi

solana program deploy target/deploy/weft.so \
  --program-id target/deploy/weft-keypair.json \
  --keypair "${ADMIN}" \
  --url "${RPC}"

echo "[deploy] done on ${CLUSTER}: ${DECLARE_ID}"
