#!/usr/bin/env bash
# Start a fresh local Solana validator for integration testing.
# Usage: ./scripts/start-localnet.sh [extra solana-test-validator args]
set -euo pipefail

exec solana-test-validator --reset "$@"
