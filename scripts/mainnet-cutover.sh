#!/usr/bin/env bash
# Weft relay VPS cutover: point the control-plane + aggregator at mainnet and the
# pump.fun reward mint, wipe leftover devnet state, restart. Run at launch AFTER
# mainnet-launch.sh has initialized the on-chain distributor.
#
#   ./scripts/mainnet-cutover.sh <MINT_CA> [--yes]
#
# What it changes on /etc/weft: node.env + aggregator.env -> WEFT_CLUSTER=mainnet-beta,
# WEFT_RPC/WS -> Helius mainnet, WEFT_MINT=<CA>. Faucet stays disabled. Stateful stores
# (users/epochs/payouts/settled/exit-profiles) are moved aside so no devnet balance,
# epoch counter, or paid-total leaks into mainnet (double-pay / stale-balance guard).
#
# The Helius key is read from the local (uncommitted) ~/Documents/helius/config.json and
# injected server-side; it is never written into this committed script.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST="${WEFT_VPS:-root@13.140.2.111}"
MINT="${1:-${WEFT_MINT:-}}"
YES=0
for a in "$@"; do [[ "$a" == "--yes" || "$a" == "-y" ]] && YES=1; done
[[ "${WEFT_LAUNCH_YES:-0}" == "1" ]] && YES=1
KEEP_STORES="${WEFT_KEEP_STORES:-0}"

if [[ -z "${MINT}" || "${MINT}" == "--yes" || "${MINT}" == "-y" ]]; then
  echo "usage: $0 <MINT_CA> [--yes]" >&2
  exit 1
fi

# --- resolve Helius mainnet RPC (local key), else explicit override, else public ---
RPC_HTTP="${WEFT_RPC_URL:-}"
if [[ -z "${RPC_HTTP}" ]]; then
  HELIUS_CFG="${HOME}/Documents/helius/config.json"
  if [[ -f "${HELIUS_CFG}" ]]; then
    KEY="$(node -e "process.stdout.write((require('${HELIUS_CFG}').apiKey)||'')" 2>/dev/null || true)"
    [[ -n "${KEY}" ]] && RPC_HTTP="https://mainnet.helius-rpc.com/?api-key=${KEY}"
  fi
fi
RPC_HTTP="${RPC_HTTP:-https://api.mainnet-beta.solana.com}"
RPC_WS="$(printf '%s' "${RPC_HTTP}" | sed -E 's|^https://|wss://|; s|^http://|ws://|')"
RPC_MASKED="$(printf '%s' "${RPC_HTTP}" | sed -E 's/(api-key=)[^&]+/\1***/')"

# --- relay secrets (Reality identity + tokens) from a LOCAL uncommitted file — never the repo ---
# The Reality private key and founder UUID are node secrets; the relay/receipts tokens gate the
# relay + aggregator. They are generated fresh (scripts/gen-relay-secrets.mjs) and live only here.
RELAY_SECRETS="${WEFT_RELAY_SECRETS:-${HOME}/.config/weft/relay-secrets.env}"
if [[ ! -f "${RELAY_SECRETS}" ]]; then
  echo "ERROR: relay secrets file ${RELAY_SECRETS} not found." >&2
  echo "       Generate it once with: node scripts/gen-relay-secrets.mjs > \"${RELAY_SECRETS}\"" >&2
  echo "       It must define WEFT_REALITY_PBK/PRIV, WEFT_SID, WEFT_FOUNDER_UUID," >&2
  echo "       WEFT_RELAY_TOKEN, WEFT_RECEIPTS_TOKEN." >&2
  exit 1
fi
# shellcheck disable=SC1090
set -a; source "${RELAY_SECRETS}"; set +a
for v in WEFT_REALITY_PBK WEFT_REALITY_PRIV WEFT_SID WEFT_FOUNDER_UUID WEFT_RELAY_TOKEN WEFT_RECEIPTS_TOKEN; do
  [[ -n "${!v:-}" ]] || { echo "ERROR: ${v} missing from ${RELAY_SECRETS}" >&2; exit 1; }
done

# The aggregator refuses to boot without an initialized distributor and would crash-loop.
# Ensure mainnet-launch.sh has already initialized on-chain state before we restart it.
# The distributor PDA is DERIVED from the program id (via the SDK), so it stays correct if
# the id is ever rotated — never hardcode it.
DISTRIBUTOR="$(cd "${ROOT}" && pnpm --filter @weft/registry-provision exec tsx \
  src/printDistributor.ts 2>/dev/null | tail -1)"
if [[ -z "${DISTRIBUTOR}" ]]; then
  echo "ERROR: could not derive the distributor PDA from @weft/sdk (run 'pnpm install && pnpm -r build' first)." >&2
  exit 1
fi
echo "[cutover] distributor PDA (derived from program id): ${DISTRIBUTOR}"
if ! solana account "${DISTRIBUTOR}" --url "${RPC_HTTP}" >/dev/null 2>&1; then
  echo "ERROR: distributor ${DISTRIBUTOR} not found on mainnet." >&2
  echo "       Run ./scripts/mainnet-launch.sh <CA> first, then re-run this cutover." >&2
  exit 1
fi

cat <<EOF

===================== WEFT MAINNET CUTOVER =====================
  vps         : ${HOST}
  cluster     : mainnet-beta
  rpc         : ${RPC_MASKED}
  reward mint : ${MINT}
  reset stores: $([[ "${KEEP_STORES}" == "1" ]] && echo "no (WEFT_KEEP_STORES=1)" || echo "yes (devnet state moved aside)")
  restarts    : weft-control-plane, weft-aggregator
=================================================================
EOF

if [[ "${YES}" != "1" ]]; then
  read -r -p "Type CUTOVER to proceed: " CONFIRM
  [[ "${CONFIRM}" == "CUTOVER" ]] || { echo "aborted."; exit 1; }
fi

TS="$(date -u +%Y%m%dT%H%M%SZ)"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

# setkv <file> <key> <value>: replace or append KEY=VALUE, preserving all other lines.
setkv() {
  local f="$1" k="$2" v="$3"
  grep -v "^${k}=" "$f" > "$f.tmp" 2>/dev/null || true
  printf '%s=%s\n' "$k" "$v" >> "$f.tmp"
  mv "$f.tmp" "$f"
}

echo "[cutover] fetching current env from ${HOST}"
scp -q "${HOST}:/etc/weft/node.env" "${WORK}/node.env"
scp -q "${HOST}:/etc/weft/aggregator.env" "${WORK}/aggregator.env"

# node.env (control-plane)
setkv "${WORK}/node.env" WEFT_CLUSTER "mainnet-beta"
setkv "${WORK}/node.env" WEFT_RPC "${RPC_HTTP}"
setkv "${WORK}/node.env" WEFT_WS "${RPC_WS}"
setkv "${WORK}/node.env" WEFT_MINT "${MINT}"
setkv "${WORK}/node.env" WEFT_FAUCET_KEYPAIR ""
# On mainnet the control-plane hard-requires these. Injected from the local secrets file above —
# never committed. The VPS Xray config must be rendered from the SAME values (the control-plane
# does this on restart), so rotating here rotates the live node identity.
setkv "${WORK}/node.env" WEFT_REALITY_PBK "${WEFT_REALITY_PBK}"
setkv "${WORK}/node.env" WEFT_REALITY_PRIV "${WEFT_REALITY_PRIV}"
setkv "${WORK}/node.env" WEFT_SID "${WEFT_SID}"
setkv "${WORK}/node.env" WEFT_FOUNDER_UUID "${WEFT_FOUNDER_UUID}"
setkv "${WORK}/node.env" WEFT_RELAY_TOKEN "${WEFT_RELAY_TOKEN}"

# aggregator.env
setkv "${WORK}/aggregator.env" WEFT_CLUSTER "mainnet-beta"
setkv "${WORK}/aggregator.env" WEFT_RPC "${RPC_HTTP}"
setkv "${WORK}/aggregator.env" WEFT_RPC_WS "${RPC_WS}"
setkv "${WORK}/aggregator.env" WEFT_RECEIPTS_TOKEN "${WEFT_RECEIPTS_TOKEN}"

echo "[cutover] NOTE: the VPS must already run the NEW program-id build. The aggregator derives the"
echo "[cutover]       distributor from the baked program id; a stale bundle points at a dead"
echo "[cutover]       distributor and will crash-loop. Update the VPS code (git pull + pnpm build, or"
echo "[cutover]       re-fetch the committed bundles) BEFORE this cutover if the id was rotated."
echo "[cutover] installing env + restarting services"
scp -q "${WORK}/node.env" "${HOST}:/tmp/node.env.new"
scp -q "${WORK}/aggregator.env" "${HOST}:/tmp/aggregator.env.new"

ssh "${HOST}" \
  "TS='${TS}' KEEP_STORES='${KEEP_STORES}' MINT='${MINT}' bash -s" <<'REMOTE'
set -euo pipefail
cd /etc/weft
cp -a node.env "node.env.bak-${TS}"
cp -a aggregator.env "aggregator.env.bak-${TS}"
mv /tmp/node.env.new node.env
mv /tmp/aggregator.env.new aggregator.env

if [[ "${KEEP_STORES}" != "1" ]]; then
  mkdir -p /var/lib/weft
  for s in users.json reward-epochs.json payouts.json settled-profile-bytes.json exit-profiles.json; do
    if [[ -f "/var/lib/weft/${s}" ]]; then
      mv "/var/lib/weft/${s}" "/var/lib/weft/${s}.devnet-bak-${TS}"
      echo "  reset store ${s}"
    fi
  done
fi

systemctl restart weft-control-plane weft-aggregator
sleep 4
echo "--- service state ---"
systemctl is-active weft-control-plane weft-aggregator || true
# Fail loudly if the aggregator didn't come up — the usual cause is a STALE program-id build on the
# VPS (it derives a distributor that doesn't exist on mainnet and crash-loops). Catch it here rather
# than declare a "successful" cutover with dead settlement.
if ! systemctl is-active --quiet weft-aggregator; then
  echo "ERROR: weft-aggregator is not active after restart." >&2
  echo "       Most likely the VPS runs a STALE program-id build (dead distributor). Update the VPS" >&2
  echo "       to the new-id build and restart before retrying the cutover." >&2
  journalctl -u weft-aggregator -n 25 --no-pager || true
  exit 1
fi
echo "--- /price ---"
curl -s --max-time 8 http://127.0.0.1:8088/price || echo "(control-plane /price not ready yet)"
echo ""
REMOTE

echo ""
echo "[cutover] done. Expected: /price shows mint=${MINT}, faucet=false."
echo "[cutover] Node rewards are claimed on-chain from the reward vault (funded by the 70% split) —"
echo "[cutover] no payout wallet to top up. Confirm the aggregator posts epochs after first traffic."
