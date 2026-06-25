#!/usr/bin/env bash
#
# Deploy a Weft VPN node on a fresh Ubuntu/Debian VPS. The node offers TWO connection modes,
# both over standard VLESS + Reality, so any stock client (Happ / V2Box / sing-box / Hiddify /
# Streisand) connects with a single pasted link:
#
#   1-HOP     — direct VLESS + Reality exit. Fast. Traffic egresses at this node.
#   MULTIHOP  — VLESS + Reality, then routed through the Tor network. Onion, max privacy,
#               slower; traffic egresses at a Tor exit, not this node.
#
# The data plane is the battle-tested Xray-core (Reality) + the Tor daemon — no custom code.
# Weft's value-add is the polished setup + the on-chain $WEFT incentive/registry layer.
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

echo "→ dependencies (tor provides the multihop egress)…"
apt-get update -y >/dev/null
DEBIAN_FRONTEND=noninteractive apt-get install -y curl tor qrencode openssl >/dev/null
systemctl enable --now tor >/dev/null 2>&1

echo "→ Xray-core (stable 1.8.24 — newer 26.x breaks the vision flow)…"
bash -c "$(curl -L -s https://github.com/XTLS/Xray-install/raw/main/install-release.sh)" @ install --version 1.8.24 >/dev/null

KEYS=$(/usr/local/bin/xray x25519)
PRIV=$(echo "$KEYS" | grep -i privat | awk '{print $NF}')
PUB=$(echo "$KEYS" | grep -iE 'public|password' | awk '{print $NF}')
UUID=$(/usr/local/bin/xray uuid)
SID=$(openssl rand -hex 8)
RS="{ \"show\": false, \"dest\": \"${SNI}:443\", \"xver\": 0, \"serverNames\": [\"${SNI}\"], \"privateKey\": \"${PRIV}\", \"shortIds\": [\"${SID}\"] }"

cat > /usr/local/etc/xray/config.json <<CONF
{
  "log": { "loglevel": "warning" },
  "inbounds": [
    { "tag": "hop1", "listen": "0.0.0.0", "port": ${HOP1_PORT}, "protocol": "vless",
      "settings": { "clients": [ { "id": "${UUID}", "flow": "xtls-rprx-vision" } ], "decryption": "none" },
      "streamSettings": { "network": "tcp", "security": "reality", "realitySettings": ${RS} } },
    { "tag": "hopN", "listen": "0.0.0.0", "port": ${HOPN_PORT}, "protocol": "vless",
      "settings": { "clients": [ { "id": "${UUID}" } ], "decryption": "none" },
      "streamSettings": { "network": "tcp", "security": "reality", "realitySettings": ${RS} } }
  ],
  "outbounds": [
    { "tag": "direct", "protocol": "freedom" },
    { "tag": "tor", "protocol": "socks", "settings": { "servers": [ { "address": "127.0.0.1", "port": 9050 } ] } }
  ],
  "routing": { "rules": [
    { "type": "field", "inboundTag": ["hop1"], "outboundTag": "direct" },
    { "type": "field", "inboundTag": ["hopN"], "outboundTag": "tor" }
  ] }
}
CONF

systemctl enable xray >/dev/null 2>&1
systemctl restart xray

HOST=$(curl -s --max-time 10 https://api.ipify.org)
H1="vless://${UUID}@${HOST}:${HOP1_PORT}?flow=xtls-rprx-vision&type=tcp&security=reality&fp=firefox&sni=${SNI}&pbk=${PUB}&sid=${SID}&spx=%2F#Weft-1hop"
HN="vless://${UUID}@${HOST}:${HOPN_PORT}?type=tcp&security=reality&fp=firefox&sni=${SNI}&pbk=${PUB}&sid=${SID}&spx=%2F#Weft-multihop"

echo
echo "✅ Weft node live (masquerading as ${SNI}). Import either link into Happ / V2Box / any VLESS client:"
echo
echo "  1-HOP (fast):      ${H1}"
echo
echo "  MULTIHOP (Tor):    ${HN}"
echo
echo "QR for the 1-hop link:"
qrencode -t ansiutf8 "${H1}"
