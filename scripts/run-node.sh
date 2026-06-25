#!/usr/bin/env bash
#
# Turn a HOME device (PC / router / always-on box) into a Weft node — under your own wallet,
# earning $WEFT for the traffic you actually serve. A home device is behind NAT, so it can't
# accept inbound connections; like Tailscale-DERP / Cloudflare-Tunnel / Tor relays, it dials OUT
# to a public **rendezvous relay** (frp) which exposes it at a public `relay:port`. Users connect
# there; the relay forwards to your home Xray; your traffic exits at home.
#
#   [user] --VLESS+Reality--> [relay:port] --frp--> [home Xray] --(1-hop)--> internet / --(multihop)--> Tor
#
# Installs xray + tor + frpc + the control plane as **persistent services** (systemd on Linux,
# launchd on macOS) so the node survives reboots, crashes, and closing the terminal. Stop it any
# time with ./scripts/stop-node.sh.
#
# Just run it — no flags, no tokens, no config:  ./scripts/run-node.sh
# Then register the endpoint it prints in the cabinet (your connected wallet = your earnings).
#
# Optional overrides (all have working defaults): WEFT_RELAY · WEFT_RELAY_PORT ·
# WEFT_RELAY_TOKEN · WEFT_SNI
set -euo pipefail

RELAY="${WEFT_RELAY:-vpn.weftnetwork.net}"
RELAY_PORT="${WEFT_RELAY_PORT:-7000}"
# The public launch relay is open (like a Tor relay) — its token is not a secret. Baked in so a
# node operator just runs the script; override WEFT_RELAY_TOKEN only when running a private relay.
TOKEN="${WEFT_RELAY_TOKEN:-a40b1ab498a37ba6bbaa70791ac62287}"
SNI="${WEFT_SNI:-www.microsoft.com}"
LOCAL_HOP1=14430
LOCAL_HOPN=18443
FRP_VER="0.69.1"
RAW="https://raw.githubusercontent.com/kerryjanes/WeftNetwork/main"
SK="$HOME/.weft"
mkdir -p "$SK"

OS=$(uname -s)
echo "→ dependencies…"
if [ "$OS" = "Linux" ]; then
  sudo apt-get update -y >/dev/null
  DEBIAN_FRONTEND=noninteractive sudo apt-get install -y curl tor openssl python3 >/dev/null
  sudo systemctl enable --now tor >/dev/null 2>&1 || true
  command -v node >/dev/null 2>&1 || { curl -fsSL https://deb.nodesource.com/setup_20.x | sudo bash - >/dev/null 2>&1; sudo apt-get install -y nodejs >/dev/null 2>&1; }
  command -v xray >/dev/null 2>&1 || sudo bash -c "$(curl -L -s https://github.com/XTLS/Xray-install/raw/main/install-release.sh)" @ install --version 1.8.24 >/dev/null
  XRAY=/usr/local/bin/xray; ARCH=linux_amd64; NODE=$(command -v node)
else
  echo "  (macOS: ensure xray, tor, node are installed via brew: brew install xray tor node)"
  XRAY=$(command -v xray); ARCH=darwin_$( [ "$(uname -m)" = "arm64" ] && echo arm64 || echo amd64 ); NODE=$(command -v node)
fi

echo "→ frpc (reverse tunnel)…"
if [ ! -x "$SK/frpc" ]; then
  curl -fsSL "https://github.com/fatedier/frp/releases/download/v${FRP_VER}/frp_${FRP_VER}_${ARCH}.tar.gz" -o "$SK/frp.tgz"
  tar xzf "$SK/frp.tgz" -C "$SK" && cp "$SK/frp_${FRP_VER}_${ARCH}/frpc" "$SK/frpc" && chmod +x "$SK/frpc" && rm -rf "$SK/frp.tgz" "$SK/frp_${FRP_VER}_${ARCH}"
fi

# Identity + a stable pair of public relay ports derived from this node's key.
KEYS=$("$XRAY" x25519); PRIV=$(echo "$KEYS"|grep -i privat|awk '{print $NF}'); PUB=$(echo "$KEYS"|grep -iE 'public|password'|awk '{print $NF}')
UUID=$("$XRAY" uuid); SID=$(openssl rand -hex 8)
SEED=$(echo "$PUB" | python3 -c "import sys,hashlib;print(int(hashlib.sha256(sys.stdin.read().strip().encode()).hexdigest(),16)%40)")
PUB_HOP1=$((20000 + SEED * 2)); PUB_HOPN=$((PUB_HOP1 + 1))

# Auto-detect GEO (packed 6-char geohash) from this node's public IP — no manual region code.
GEO=$(python3 - <<'PY'
import json,urllib.request
try:
    d=json.load(urllib.request.urlopen("http://ip-api.com/json/?fields=lat,lon",timeout=8))
    lat,lon=float(d["lat"]),float(d["lon"])
except Exception:
    print(0); raise SystemExit
B32="0123456789bcdefghjkmnpqrstuvwxyz"
def geohash(lat,lon,prec=6):
    latr=[-90.0,90.0]; lonr=[-180.0,180.0]; bits=[16,8,4,2,1]; out=[]; even=True; bit=0; ch=0
    while len(out)<prec:
        if even:
            mid=(lonr[0]+lonr[1])/2
            if lon>mid: ch|=bits[bit]; lonr[0]=mid
            else: lonr[1]=mid
        else:
            mid=(latr[0]+latr[1])/2
            if lat>mid: ch|=bits[bit]; latr[0]=mid
            else: latr[1]=mid
        even=not even
        if bit<4: bit+=1
        else: out.append(B32[ch]); bit=0; ch=0
    return "".join(out)
gh=geohash(lat,lon)
packed=0
for c in gh: packed=packed*32+B32.index(c)
print(packed)
PY
)
GEO="${GEO:-0}"

echo "→ configs…"
cat > "$SK/frpc.toml" <<TOML
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

curl -fsSL "${RAW}/services/control-plane/dist/control-plane.mjs" -o "$SK/control-plane.mjs"

if [ "$OS" = "Linux" ]; then
  RELOAD="systemctl restart weft-node-xray"
else
  RELOAD="launchctl kickstart -k gui/$(id -u)/com.weft.node.xray"
fi
cat > "$SK/node.env" <<ENV
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
WEFT_GEO=${GEO}
WEFT_XRAY_CONFIG=${SK}/xray.json
WEFT_XRAY_RELOAD=${RELOAD}
WEFT_STORE=${SK}/users.json
WEFT_PORT=8088
ENV

echo "→ install persistent services (survive reboot + auto-restart)…"
if [ "$OS" = "Linux" ]; then
  unit() { sudo tee "/etc/systemd/system/$1.service" >/dev/null; }
  echo "[Unit]
Description=Weft node — xray
After=network-online.target
[Service]
ExecStart=${XRAY} run -c ${SK}/xray.json
Restart=always
RestartSec=3
[Install]
WantedBy=multi-user.target" | unit weft-node-xray
  echo "[Unit]
Description=Weft node — frpc reverse tunnel
After=network-online.target
[Service]
ExecStart=${SK}/frpc -c ${SK}/frpc.toml
Restart=always
RestartSec=3
[Install]
WantedBy=multi-user.target" | unit weft-node-frpc
  echo "[Unit]
Description=Weft node — control plane
After=network-online.target
[Service]
EnvironmentFile=${SK}/node.env
ExecStart=${NODE} ${SK}/control-plane.mjs
Restart=always
RestartSec=3
[Install]
WantedBy=multi-user.target" | unit weft-node-cp
  sudo systemctl daemon-reload
  sudo systemctl enable --now weft-node-cp     # writes xray.json + (re)starts xray
  sudo systemctl enable --now weft-node-frpc
  sudo systemctl enable weft-node-xray
else
  LA="$HOME/Library/LaunchAgents"; mkdir -p "$LA"
  plist() { cat > "$LA/com.weft.node.$1.plist"; launchctl load -w "$LA/com.weft.node.$1.plist" 2>/dev/null || true; }
  printf '<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>com.weft.node.xray</string><key>ProgramArguments</key><array><string>%s</string><string>run</string><string>-c</string><string>%s/xray.json</string></array><key>KeepAlive</key><true/><key>RunAtLoad</key><true/></dict></plist>' "$XRAY" "$SK" | plist xray
  printf '<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>com.weft.node.frpc</string><key>ProgramArguments</key><array><string>%s/frpc</string><string>-c</string><string>%s/frpc.toml</string></array><key>KeepAlive</key><true/><key>RunAtLoad</key><true/></dict></plist>' "$SK" "$SK" | plist frpc
  printf '<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>com.weft.node.cp</string><key>ProgramArguments</key><array><string>%s</string><string>%s/control-plane.mjs</string></array><key>EnvironmentVariables</key><dict><key>WEFT_ENVFILE</key><string>%s/node.env</string></dict><key>KeepAlive</key><true/><key>RunAtLoad</key><true/></dict></plist>' "$NODE" "$SK" "$SK" | plist cp
fi

sleep 4
cat <<DONE

✅ Weft home node is up (persistent — survives reboot, crash, and closing the terminal).
   Public endpoint:  ${RELAY}:${PUB_HOP1}   ·   geo (auto): ${GEO}
   Control plane:    http://127.0.0.1:8088

NEXT — register it on-chain to start earning \$WEFT:
   open the cabinet → connect your wallet → "deploy" → paste:  ${RELAY}:${PUB_HOP1}

Stop the node any time:  ./scripts/stop-node.sh
DONE
