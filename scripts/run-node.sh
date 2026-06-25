#!/usr/bin/env bash
#
# Turn a HOME device (PC / router / always-on box) into a Weft node — under your own wallet,
# earning $WEFT for the traffic you actually serve. Unlike deploy-node.sh (a public VPS with its
# own IP), a home device is behind NAT, so it can't accept inbound connections directly. This
# agent solves that exactly like Tailscale-DERP / Cloudflare-Tunnel / Tor relays do: it dials OUT
# to a public **rendezvous relay** (frp) which exposes your node at a public `relay:port`. Users
# connect to that public address; the relay forwards to your home Xray; your traffic exits at home.
#
#   [user] --Reality--> [relay:port] --frp tunnel--> [your home Xray] --> exit
#
# Stack (all battle-tested, no custom data-plane code): Xray-core (VLESS+Reality) + Tor (multihop)
# + frpc (reverse tunnel) + the Weft control plane (meters your served traffic, gates by $WEFT).
#
# Usage (Ubuntu/Debian/macOS, as a user who can sudo):
#   WEFT_RELAY_TOKEN=<token>  ./scripts/run-node.sh
# Env:
#   WEFT_RELAY        rendezvous host (default vpn.weftnetwork.net)
#   WEFT_RELAY_PORT   rendezvous frps bind port (default 7000)
#   WEFT_RELAY_TOKEN  rendezvous auth token (REQUIRED — ask the relay operator)
#   WEFT_SNI          Reality SNI to masquerade as (default www.microsoft.com; ya.ru for RU)
#   WEFT_WALLET       your Solana pubkey (for the operator/earnings identity in the cabinet)
set -euo pipefail

RELAY="${WEFT_RELAY:-vpn.weftnetwork.net}"
RELAY_PORT="${WEFT_RELAY_PORT:-7000}"
TOKEN="${WEFT_RELAY_TOKEN:?set WEFT_RELAY_TOKEN (the rendezvous auth token)}"
SNI="${WEFT_SNI:-www.microsoft.com}"
WALLET="${WEFT_WALLET:-}"
LOCAL_HOP1=14430   # local Xray listen ports (loopback; frpc forwards the relay onto these)
LOCAL_HOPN=18443
FRP_VER="0.69.1"
RAW="https://raw.githubusercontent.com/kerryjanes/WeftNetwork/main"

uname_s=$(uname -s)
echo "→ dependencies…"
if [ "$uname_s" = "Linux" ]; then
  sudo apt-get update -y >/dev/null
  DEBIAN_FRONTEND=noninteractive sudo apt-get install -y curl tor openssl python3 >/dev/null
  sudo systemctl enable --now tor >/dev/null 2>&1 || true
  command -v node >/dev/null 2>&1 || { curl -fsSL https://deb.nodesource.com/setup_20.x | sudo bash - >/dev/null 2>&1; sudo apt-get install -y nodejs >/dev/null 2>&1; }
  command -v xray >/dev/null 2>&1 || sudo bash -c "$(curl -L -s https://github.com/XTLS/Xray-install/raw/main/install-release.sh)" @ install --version 1.8.24 >/dev/null
  XRAY=/usr/local/bin/xray
  ARCH=linux_amd64
else
  echo "  (macOS: ensure xray, tor, node are installed via brew)"
  XRAY=$(command -v xray)
  ARCH=darwin_$( [ "$(uname -m)" = "arm64" ] && echo arm64 || echo amd64 )
fi

mkdir -p "$HOME/.weft"
cd "$HOME/.weft"

echo "→ frpc (reverse tunnel to the rendezvous)…"
if [ ! -x ./frpc ]; then
  curl -fsSL "https://github.com/fatedier/frp/releases/download/v${FRP_VER}/frp_${FRP_VER}_${ARCH}.tar.gz" -o frp.tgz
  tar xzf frp.tgz && cp "frp_${FRP_VER}_${ARCH}/frpc" ./frpc && chmod +x ./frpc && rm -rf frp.tgz "frp_${FRP_VER}_${ARCH}"
fi

# Identity + a stable pair of public relay ports derived from this node's key (low collision).
KEYS=$("$XRAY" x25519)
PRIV=$(echo "$KEYS" | grep -i privat | awk '{print $NF}')
PUB=$(echo "$KEYS" | grep -iE 'public|password' | awk '{print $NF}')
UUID=$("$XRAY" uuid)
SID=$(openssl rand -hex 8)
SEED=$(echo "${PUB}" | python3 -c "import sys,hashlib;print(int(hashlib.sha256(sys.stdin.read().strip().encode()).hexdigest(),16)%40)")
PUB_HOP1=$((20000 + SEED * 2))
PUB_HOPN=$((PUB_HOP1 + 1))

echo "→ frpc config (relay ${RELAY}:${PUB_HOP1} → local ${LOCAL_HOP1}, +multihop)…"
cat > frpc.toml <<TOML
serverAddr = "${RELAY}"
serverPort = ${RELAY_PORT}
auth.method = "token"
auth.token = "${TOKEN}"
[[proxies]]
name = "weft-1hop-${UUID:0:8}"
type = "tcp"
localIP = "127.0.0.1"
localPort = ${LOCAL_HOP1}
remotePort = ${PUB_HOP1}
[[proxies]]
name = "weft-multihop-${UUID:0:8}"
type = "tcp"
localIP = "127.0.0.1"
localPort = ${LOCAL_HOPN}
remotePort = ${PUB_HOPN}
TOML

echo "→ Weft control plane (meters your served traffic, gates users by \$WEFT)…"
curl -fsSL "${RAW}/services/control-plane/dist/control-plane.mjs" -o control-plane.mjs
cat > node.env <<ENV
WEFT_HOST=${RELAY}
WEFT_REALITY_PBK=${PUB}
WEFT_REALITY_PRIV=${PRIV}
WEFT_SID=${SID}
WEFT_SNI=${SNI}
WEFT_HOP1_PORT=${LOCAL_HOP1}
WEFT_HOPN_PORT=${LOCAL_HOPN}
WEFT_PUBLIC_HOP1_PORT=${PUB_HOP1}
WEFT_PUBLIC_HOPN_PORT=${PUB_HOPN}
WEFT_FOUNDER_UUID=${UUID}
WEFT_XRAY_CONFIG=${HOME}/.weft/xray.json
WEFT_XRAY_RELOAD=${HOME}/.weft/reload-xray.sh
WEFT_STORE=${HOME}/.weft/users.json
WEFT_PORT=8088
ENV

# A home node manages its OWN xray config file + reload (not the system service).
cat > reload-xray.sh <<RLD
#!/usr/bin/env bash
pkill -f "xray run -c ${HOME}/.weft/xray.json" 2>/dev/null || true
nohup "${XRAY}" run -c "${HOME}/.weft/xray.json" >${HOME}/.weft/xray.log 2>&1 &
RLD
chmod +x reload-xray.sh

echo "→ launch frpc + control plane…"
nohup ./frpc -c frpc.toml >frpc.log 2>&1 &
nohup node control-plane.mjs >control-plane.log 2>&1 &
sleep 4

cat <<DONE

✅ Weft home node is up.
   Public endpoint (via relay):  ${RELAY}:${PUB_HOP1}   (1-hop) · ${RELAY}:${PUB_HOPN} (multihop)
   Control plane:                http://127.0.0.1:8088   (users provision personal links here)

NEXT — register this node on-chain to start EARNING \$WEFT for traffic you serve:
   • Open the cabinet, connect your wallet, go to "deploy", and register with endpoint:
         ${RELAY}:${PUB_HOP1}
   • Or run, with your operator keypair:
         WEFT_NODE_ENDPOINT=${RELAY}:${PUB_HOP1} pnpm --filter @weft/registry-provision node:register

Your node now serves both modes; the control plane meters each user and the network pays you in
\$WEFT proportionally to the traffic that actually passes through you.
DONE
