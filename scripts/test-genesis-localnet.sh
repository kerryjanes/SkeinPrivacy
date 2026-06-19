#!/usr/bin/env bash
# Start a local validator with weft-vesting deployed, fund the deployer, and run
# the genesis integration test. Tears the validator down on exit.
set -euo pipefail
cd "$(dirname "$0")/.."

PROGRAM_ID=FCFZNb2Kqh7ScjikKp73W7BcsfusrZ1hTBhc61Macdsv
SO=target/deploy/weft_vesting.so
[[ -f "$SO" ]] || { echo "Build the program first: anchor build"; exit 1; }

pkill -f solana-test-validator 2>/dev/null || true
sleep 1
rm -rf test-ledger
solana-test-validator --reset --quiet --bpf-program "$PROGRAM_ID" "$SO" >test-ledger.log 2>&1 &
trap 'kill %1 2>/dev/null || true' EXIT

for _ in $(seq 1 30); do
  solana cluster-version --url http://127.0.0.1:8899 >/dev/null 2>&1 && break
  sleep 1
done
solana airdrop 100 "$(solana address)" --url http://127.0.0.1:8899 >/dev/null

WEFT_CLUSTER=localnet WEFT_RPC_URL=http://127.0.0.1:8899 pnpm -F @weft/genesis exec vitest run
