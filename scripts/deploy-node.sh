#!/usr/bin/env bash
#
# Deploy a Weft VPN node on a fresh Ubuntu/Debian VPS. A node offers TWO connection modes over
# standard VLESS + Reality, so any stock client (Happ / V2Box / sing-box / Hiddify / Streisand)
# connects with a single pasted link:
#
#   1-HOP     — direct VLESS + Reality exit. Fast. Traffic egresses at this node.
#   MULTIHOP  — VLESS + Reality, then routed through the Tor network. Onion, max privacy, slower.
#
# Access is METERED and gated by $WEFT: the Weft control plane (installed here) mints a personal
# link per wallet, meters its traffic via xray's stats API, and cuts a user off once they've used
# more than their $WEFT balance pays for (1000 WEFT/GB) — restoring them when they top up. The
# always-on "founder" link stays unmetered so the operator can always reach the node.
#
# Data plane = battle-tested Xray-core (Reality) + the Tor daemon. Control plane = Node.js service
# that owns the xray config + talks to Solana. The control plane renders /usr/local/etc/xray/
# config.json itself, so this script does NOT write it directly.
#
# Usage (as root):
#   ./scripts/deploy-node.sh [reality-sni]
#     reality-sni: a real TLS1.3 site to masquerade as (default www.microsoft.com). For markets
#     with heavy DPI (e.g. Russia), use a DOMESTIC domain (e.g. ya.ru) — foreign SNIs get flagged.
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "run as root"; exit 1; }

SNI="${1:-www.microsoft.com}"
HOP1_PORT="${HOP1_PORT:-443}"
HOPN_PORT="${HOPN_PORT:-8443}"
API_PORT="${API_PORT:-8088}"
CLUSTER="${WEFT_CLUSTER:-devnet}"
RPC="${WEFT_RPC:-https://api.devnet.solana.com}"
WEFT_MINT="${WEFT_MINT:-Hfvwx9F5NDzMCyywJZJsFVX83XaXnLNntCdk21h7Bmcy}"
if [[ "$CLUSTER" == mainnet* ]]; then
  [ -n "${WEFT_RPC:-}" ] || { echo "WEFT_RPC must be set explicitly for ${CLUSTER}"; exit 1; }
  [ -n "${WEFT_MINT:-}" ] || { echo "WEFT_MINT must be set explicitly for ${CLUSTER}"; exit 1; }
fi
RAW="https://raw.githubusercontent.com/kerryjanes/WeftNetwork/main"

echo "→ dependencies (tor = multihop egress; node = control plane)…"
apt-get update -y >/dev/null
DEBIAN_FRONTEND=noninteractive apt-get install -y curl tor qrencode openssl python3 >/dev/null
systemctl enable --now tor >/dev/null 2>&1
if ! command -v node >/dev/null 2>&1; then
  curl -fsSL https://deb.nodesource.com/setup_20.x | bash - >/dev/null 2>&1
  DEBIAN_FRONTEND=noninteractive apt-get install -y nodejs >/dev/null 2>&1
fi

echo "→ Xray-core (stable 1.8.24 — newer 26.x breaks the vision flow)…"
bash -c "$(curl -L -s https://github.com/XTLS/Xray-install/raw/main/install-release.sh)" @ install --version 1.8.24 >/dev/null

KEYS=$(/usr/local/bin/xray x25519)
PRIV=$(echo "$KEYS" | grep -i privat | awk '{print $NF}')
PUB=$(echo "$KEYS" | grep -iE 'public|password' | awk '{print $NF}')
FOUNDER_UUID=$(/usr/local/bin/xray uuid)
SID=$(openssl rand -hex 8)
HOST=$(curl -s --max-time 10 https://api.ipify.org)

echo "→ Weft control plane…"
mkdir -p /opt/weft /etc/weft /var/lib/weft
curl -fsSL "${RAW}/services/control-plane/dist/control-plane.mjs" -o /opt/weft/control-plane.mjs

cat > /etc/weft/node.env <<ENV
WEFT_HOST=${HOST}
WEFT_REALITY_PBK=${PUB}
WEFT_REALITY_PRIV=${PRIV}
WEFT_SID=${SID}
WEFT_SNI=${SNI}
WEFT_HOP1_PORT=${HOP1_PORT}
WEFT_HOPN_PORT=${HOPN_PORT}
WEFT_FOUNDER_UUID=${FOUNDER_UUID}
WEFT_PORT=${API_PORT}
WEFT_XRAY_API=127.0.0.1:10085
WEFT_STORE=/var/lib/weft/users.json
WEFT_CLUSTER=${CLUSTER}
WEFT_RPC=${RPC}
WEFT_MINT=${WEFT_MINT}
ENV

cat > /etc/systemd/system/weft-control-plane.service <<UNIT
[Unit]
Description=Weft control plane (token-gated VPN access)
After=network-online.target xray.service
Wants=xray.service
[Service]
EnvironmentFile=/etc/weft/node.env
ExecStart=/usr/bin/node /opt/weft/control-plane.mjs
Restart=on-failure
RestartSec=3
[Install]
WantedBy=multi-user.target
UNIT

systemctl enable xray >/dev/null 2>&1
systemctl daemon-reload
systemctl enable --now weft-control-plane   # renders the xray config + starts metering
sleep 3
systemctl restart xray

H1="vless://${FOUNDER_UUID}@${HOST}:${HOP1_PORT}?flow=xtls-rprx-vision&type=tcp&security=reality&fp=firefox&sni=${SNI}&pbk=${PUB}&sid=${SID}&spx=%2F#Weft-1hop"
HN="vless://${FOUNDER_UUID}@${HOST}:${HOPN_PORT}?type=tcp&security=reality&fp=firefox&sni=${SNI}&pbk=${PUB}&sid=${SID}&spx=%2F#Weft-multihop"

echo
echo "✅ Weft node live (masquerading as ${SNI}). Control plane: http://${HOST}:${API_PORT}"
echo
echo "Users get a PERSONAL link by POSTing their wallet to the control plane:"
echo "   curl -X POST http://${HOST}:${API_PORT}/provision -d '{\"wallet\":\"<PUBKEY>\"}'"
echo "Access is metered + gated by their \$WEFT balance (1000 WEFT/GB)."
echo
echo "Operator (founder) links — unmetered, always on:"
echo "  1-HOP (fast):   ${H1}"
echo "  MULTIHOP (Tor): ${HN}"
echo
echo "QR for the founder 1-hop link:"
qrencode -t ansiutf8 "${H1}"
