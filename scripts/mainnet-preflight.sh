#!/usr/bin/env bash
# Mainnet deploy gate. This script does not send mainnet transactions.
# It verifies that the repo is in a deployable single-program shape and that
# the mainnet cabinet build does not expose devnet-only actions.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB_DIR="${WEFT_WEB_DIR:-${ROOT}/../weft-web}"
MAX_PROGRAM_RENT_SOL="${MAX_PROGRAM_RENT_SOL:-4.5}"
MAINNET_PROGRAM_ID="${WEFT_MAINNET_PROGRAM_ID:-}"

cd "${ROOT}"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

warn() {
  echo "WARN: $*" >&2
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

need anchor
need solana
need node
need pnpm
need rg
need wc

echo "== toolchain =="
anchor --version
solana --version
rustc --version

echo "== program layout =="
active_programs=()
while IFS= read -r program_manifest; do
  active_programs+=("${program_manifest}")
done < <(find programs -mindepth 2 -maxdepth 2 -name Cargo.toml | sort)
[[ "${#active_programs[@]}" == "1" ]] || fail "expected exactly one active program under programs/, got ${#active_programs[@]}"
[[ "${active_programs[0]}" == "programs/weft/Cargo.toml" ]] || fail "active program must be programs/weft"
rg -n 'members = \["crates/\*", "programs/weft"\]' Cargo.toml >/dev/null \
  || fail "Cargo workspace must include only crates/* and programs/weft"
echo "OK: single active program"

echo "== program id sync =="
anchor_id="$(anchor keys list | awk '/^weft:/ {print $2}')"
declare_id="$(rg -o 'declare_id!\("[^"]+"\)' programs/weft/src/lib.rs | sed -E 's/.*"([^"]+)".*/\1/')"
[[ -n "${anchor_id}" ]] || fail "anchor keys list did not return a weft id"
[[ "${anchor_id}" == "${declare_id}" ]] || fail "Anchor.toml/keypair id ${anchor_id} != declare_id ${declare_id}"
if [[ -n "${MAINNET_PROGRAM_ID}" ]]; then
  [[ "${declare_id}" == "${MAINNET_PROGRAM_ID}" ]] \
    || fail "WEFT_MAINNET_PROGRAM_ID=${MAINNET_PROGRAM_ID} but source/Anchor.toml use ${declare_id}"
  rg -n '^\[programs\.mainnet\]' Anchor.toml >/dev/null || fail "Anchor.toml is missing [programs.mainnet]"
  rg -n "weft = \"${MAINNET_PROGRAM_ID}\"" Anchor.toml >/dev/null \
    || fail "Anchor.toml [programs.mainnet] does not contain ${MAINNET_PROGRAM_ID}"
  echo "OK: mainnet program id configured"
else
  warn "WEFT_MAINNET_PROGRAM_ID is not set; final mainnet deploy is still blocked on choosing the mainnet program id"
fi

echo "== builds =="
NO_DNA=1 anchor build --ignore-keys -p weft >/tmp/weft-anchor-build.log
pnpm -r build >/tmp/weft-pnpm-build.log
echo "OK: anchor + TypeScript builds"

echo "== deploy rent =="
so="target/deploy/weft.so"
[[ -f "${so}" ]] || fail "missing ${so}"
bytes="$(wc -c < "${so}" | tr -d ' ')"
rent_output="$(solana rent "${bytes}")"
rent_sol="$(printf '%s\n' "${rent_output}" | awk '/Rent-exempt minimum:/ {print $3}')"
printf 'program bytes: %s\n%s\n' "${bytes}" "${rent_output}"
node -e "const rent=Number(process.argv[1]); const max=Number(process.argv[2]); if (!Number.isFinite(rent) || rent > max) process.exit(1)" \
  "${rent_sol}" "${MAX_PROGRAM_RENT_SOL}" \
  || fail "program rent ${rent_sol} SOL exceeds MAX_PROGRAM_RENT_SOL=${MAX_PROGRAM_RENT_SOL}"
echo "OK: program rent <= ${MAX_PROGRAM_RENT_SOL} SOL"

if [[ -d "${WEB_DIR}" ]]; then
  echo "== mainnet cabinet build =="
  (
    cd "${WEB_DIR}"
    VITE_WEFT_CLUSTER=mainnet-beta \
      VITE_WEFT_RPC_URL="${VITE_WEFT_RPC_URL:-https://api.mainnet-beta.solana.com}" \
      VITE_WEFT_AGGREGATOR_URL="${VITE_WEFT_AGGREGATOR_URL:-https://vpn.weftnetwork.net:8089/aggregator}" \
      VITE_WEFT_CONTROL_PLANE_URL="${VITE_WEFT_CONTROL_PLANE_URL:-https://vpn.weftnetwork.net:8089}" \
      VITE_WEFT_RELAY_HOST="${VITE_WEFT_RELAY_HOST:-vpn.weftnetwork.net}" \
      pnpm --filter @weft/web build >/tmp/weft-web-mainnet-build.log
    rg -n 'VITE_WEFT_CLUSTER:"mainnet-beta"' cabinet/dist/assets >/dev/null \
      || fail "mainnet cabinet bundle did not embed VITE_WEFT_CLUSTER=mainnet-beta"
    rg -n 'VITE_WEFT_RPC_URL:"http' cabinet/dist/assets >/dev/null \
      || fail "mainnet cabinet bundle did not embed an explicit RPC URL"
    if rg -o 'VITE_WEFT_RPC_URL:"[^"]*"' cabinet/dist/assets | rg -qi 'devnet'; then
      fail "mainnet cabinet bundle resolves VITE_WEFT_RPC_URL to a devnet endpoint"
    fi
    # Dev affordances are gated on !PRODUCTION_UI (a superset of !IS_MAINNET: on mainnet
    # PRODUCTION_UI is always on, so the keypair-paste login + faucet buttons are hidden).
    rg -n 'ALLOW_DEV_CONNECT = !PRODUCTION_UI' cabinet/src/lib/config.ts >/dev/null \
      || fail "dev keypair connect is not gated on !PRODUCTION_UI"
    rg -n '!PRODUCTION_UI' cabinet/src/panels/Access.tsx >/dev/null \
      || fail "dev faucet buttons are not gated on !PRODUCTION_UI"
  )
  echo "OK: mainnet cabinet env + dev-only UI gates"
else
  warn "web repo not found at ${WEB_DIR}; skipped cabinet mainnet build"
fi

echo "PASS: mainnet preflight checks completed"
