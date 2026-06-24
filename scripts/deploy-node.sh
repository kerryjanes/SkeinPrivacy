#!/usr/bin/env bash
#
# Deploy a public Weft entry node on a fresh Ubuntu/Debian VPS, fronted by a real
# Let's Encrypt TLS certificate on port 443 — so phones connect on standard settings,
# exactly like a normal VPN (the connection looks like ordinary HTTPS).
#
# Before running: point an A record for <domain> at this server's public IP.
#
# Usage (as root):
#   ./scripts/deploy-node.sh vpn.example.com
#
set -euo pipefail
DOMAIN="${1:?usage: deploy-node.sh <domain whose A record points at this server>}"

if [ "$(id -u)" -ne 0 ]; then echo "run as root (needs port 443 + cert)"; exit 1; fi

# A small (1–2 GB) VPS needs swap to compile Rust without running out of memory.
RAM_MB="$(free -m | awk '/^Mem:/{print $2}')"
if [ "${RAM_MB:-0}" -lt 3000 ] && [ ! -f /swapfile ]; then
  echo "→ Adding 4 GB swap (small RAM)…"
  fallocate -l 4G /swapfile 2>/dev/null || dd if=/dev/zero of=/swapfile bs=1M count=4096
  chmod 600 /swapfile && mkswap /swapfile >/dev/null && swapon /swapfile
fi

echo "→ Installing dependencies…"
apt-get update -y
apt-get install -y curl git build-essential pkg-config libssl-dev socat

if ! command -v cargo >/dev/null 2>&1; then
  echo "→ Installing Rust…"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
. "$HOME/.cargo/env"

echo "→ Fetching Weft…"
[ -d /opt/weft ] || git clone https://github.com/kerryjanes/WeftNetwork.git /opt/weft
cd /opt/weft && git pull --ff-only || true
echo "→ Building (a few minutes on a small server)…"
# Disable LTO + use more codegen units: much lighter on RAM/CPU for a small VPS.
CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
  cargo build -p weft-vpn --release

echo "→ Getting a real TLS certificate for ${DOMAIN} (Let's Encrypt)…"
curl -s https://get.acme.sh | sh -s email="admin@${DOMAIN}"
~/.acme.sh/acme.sh --set-default-ca --server letsencrypt
~/.acme.sh/acme.sh --issue --standalone -d "${DOMAIN}"
install -d /etc/weft/tls
~/.acme.sh/acme.sh --install-cert -d "${DOMAIN}" \
  --fullchain-file /etc/weft/tls/fullchain.pem \
  --key-file       /etc/weft/tls/key.pem

# A fixed UUID for this node (clients connect with it).
UUID_FILE=/etc/weft/uuid
[ -f "$UUID_FILE" ] || cat /proc/sys/kernel/random/uuid > "$UUID_FILE"
UUID="$(cat "$UUID_FILE")"

echo "→ Installing the gateway as a systemd service on :443…"
cat > /etc/systemd/system/weft-gateway.service <<UNIT
[Unit]
Description=Weft VLESS entry gateway
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/opt/weft/target/release/weft-vpn vless 0.0.0.0:443 --uuid ${UUID} \\
  --tls-cert /etc/weft/tls/fullchain.pem --tls-key /etc/weft/tls/key.pem --tls-sni ${DOMAIN}
Restart=always
RestartSec=3
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable --now weft-gateway

echo
echo "✅ Weft entry node is live on ${DOMAIN}:443 (real TLS)."
echo "   Connection link (paste into V2Box / Happ — standard settings, valid cert):"
echo
echo "   vless://${UUID}@${DOMAIN}:443?type=tcp&security=tls&sni=${DOMAIN}#Weft"
echo
echo "   Logs:   journalctl -u weft-gateway -f"
