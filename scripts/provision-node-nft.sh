#!/usr/bin/env bash
# One-time node-cNFT provisioning: create the MPL-Core "Weft Nodes" collection
# (owned by the registry PDA) + a public Bubblegum V2 merkle tree, then point the
# on-chain registry at them (set_registry_collection + register_tree). Idempotent.
#
# Prerequisites:
#   - Weft core already initialized on the cluster (scripts run `core:init`).
#   - A funded deployer keypair (the registry authority) at WEFT_KEYPAIR.
#
# Usage (devnet):   ./scripts/provision-node-nft.sh
# Usage (mainnet):  WEFT_CLUSTER=mainnet WEFT_RPC_URL=<url> ./scripts/provision-node-nft.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

# Load .env if present (WEFT_RPC_URL / WEFT_KEYPAIR / WEFT_CLUSTER).
if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

CLUSTER="${WEFT_CLUSTER:-devnet}"
echo "Provisioning node-cNFT collection + tree on ${CLUSTER}..."
if [[ "${CLUSTER}" == mainnet* && -z "${WEFT_RPC_URL:-}" ]]; then
  echo "WEFT_RPC_URL must be set explicitly for ${CLUSTER}" >&2
  exit 1
fi

pnpm --filter @weft/registry-provision nft:provision
echo "Done. The manifest (services/registry-provision/manifests/${CLUSTER}.json) records the collection + tree."
