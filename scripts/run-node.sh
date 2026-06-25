#!/usr/bin/env bash
#
# Turn a HOME device (PC / router / always-on box) into a Weft **1-hop node**, earning $WEFT for
# the traffic you serve. You register + pay ONCE on the website (cabinet → deploy) and copy your
# node key from there; this script just brings the node up under that registration. A home device is
# behind NAT, so it can't accept inbound connections; like Tailscale-DERP / Cloudflare-Tunnel / Tor
# relays it dials OUT to a public **rendezvous relay** (frp), which exposes it at a public relay:port.
# Users connect there; the relay forwards to your home Xray; your traffic exits at home (1-hop).
#
#   [user] --VLESS+Reality--> [relay:port] --frp--> [home Xray] --(1-hop)--> internet
#
# (Multihop is always served over Tor by infrastructure nodes — your device never becomes a Tor node.)
#
# Installs xray + frpc + the control plane as **persistent services** (systemd on Linux, launchd on
# macOS) so the node survives reboots, crashes, and closing the terminal.
#
# Usage:
#   First run:   ./scripts/run-node.sh <your-node-key>     # the key copied from cabinet → deploy
#   After that:  ./scripts/run-node.sh                     # the key is saved; no need to pass it
#   Stop:        ./scripts/stop-node.sh                    # stop being a node (no key needed)
#
# The node key already contains the identity, region, and relay port — there is nothing else to type.
# Registration + payment happened on the website; this script never touches the chain.
set -euo pipefail

SK="$HOME/.weft"
mkdir -p "$SK"
TOKEN="${1:-${WEFT_NODE_KEY:-}}"

OS=$(uname -s)
echo "→ dependencies…"
if [ "$OS" = "Linux" ]; then
  sudo apt-get update -y >/dev/null
  DEBIAN_FRONTEND=noninteractive sudo apt-get install -y curl openssl python3 >/dev/null
  command -v node >/dev/null 2>&1 || { curl -fsSL https://deb.nodesource.com/setup_20.x | sudo bash - >/dev/null 2>&1; sudo apt-get install -y nodejs >/dev/null 2>&1; }
  command -v xray >/dev/null 2>&1 || sudo bash -c "$(curl -L -s https://github.com/XTLS/Xray-install/raw/main/install-release.sh)" @ install --version 1.8.24 >/dev/null
  XRAY=/usr/local/bin/xray; ARCH=linux_amd64; NODE=$(command -v node)
else
  echo "  (macOS: ensure xray, node are installed via brew: brew install xray node)"
  XRAY=$(command -v xray); ARCH=darwin_$( [ "$(uname -m)" = "arm64" ] && echo arm64 || echo amd64 ); NODE=$(command -v node)
fi

# --- node identity: decode the key (first run) or reuse the saved one (restart) -------------------
if [ -n "$TOKEN" ]; then
  python3 - "$TOKEN" > "$SK/node.json" <<'PY'
import sys, base64, json
t = sys.argv[1]
raw = base64.b64decode(t + "=" * (-len(t) % 4))
d = json.loads(raw)
for k in ("realityPriv", "realityPub", "uuid", "sid", "port", "relay", "geo"):
    if k not in d:
        sys.exit(f"node key is missing '{k}' — copy it again from the cabinet → deploy page")
json.dump(d, sys.stdout)
PY
elif [ ! -f "$SK/node.json" ]; then
  echo "no node key. Register your device on the website (cabinet → deploy), copy the key, then:"
  echo "  ./scripts/run-node.sh <your-node-key>"
  exit 1
fi

eval "$(python3 - "$SK/node.json" <<'PY'
import sys, json
d = json.load(open(sys.argv[1]))
print(f'PRIV={d["realityPriv"]}')
print(f'PUB={d["realityPub"]}')
print(f'UUID={d["uuid"]}')
print(f'SID={d["sid"]}')
print(f'PORT={int(d["port"])}')
print(f'RELAY={d["relay"]}')
print(f'GEO={int(d["geo"])}')
PY
)"

RELAY="${WEFT_RELAY:-$RELAY}"
RELAY_PORT="${WEFT_RELAY_PORT:-7000}"
# The public launch relay is open (like a Tor relay) — its token is not a secret. Baked in so a node
# operator just runs the script; override WEFT_RELAY_TOKEN only when running a private relay.
RELAY_TOKEN="${WEFT_RELAY_TOKEN:-a40b1ab498a37ba6bbaa70791ac62287}"
SNI="${WEFT_SNI:-ya.ru}"
LOCAL_HOP1=14430
FRP_VER="0.69.1"
RAW="https://raw.githubusercontent.com/kerryjanes/WeftNetwork/main"

echo "→ frpc (reverse tunnel)…"
if [ ! -x "$SK/frpc" ]; then
  curl -fsSL "https://github.com/fatedier/frp/releases/download/v${FRP_VER}/frp_${FRP_VER}_${ARCH}.tar.gz" -o "$SK/frp.tgz"
  tar xzf "$SK/frp.tgz" -C "$SK" && cp "$SK/frp_${FRP_VER}_${ARCH}/frpc" "$SK/frpc" && chmod +x "$SK/frpc" && rm -rf "$SK/frp.tgz" "$SK/frp_${FRP_VER}_${ARCH}"
fi

echo "→ configs…"
cat > "$SK/frpc.toml" <<TOML
serverAddr = "${RELAY}"
serverPort = ${RELAY_PORT}
auth.method = "token"
auth.token = "${RELAY_TOKEN}"
[[proxies]]
name = "weft-1hop-${UUID:0:8}"
type = "tcp"
localIP = "127.0.0.1"
localPort = ${LOCAL_HOP1}
remotePort = ${PORT}
TOML

curl -fsSL "${RAW}/services/control-plane/dist/control-plane.mjs" -o "$SK/control-plane.mjs"

if [ "$OS" = "Linux" ]; then
  RELOAD="systemctl restart weft-node-xray"
else
  RELOAD="launchctl kickstart -k gui/$(id -u)/com.weft.node.xray"
fi
# 1-hop ONLY: WEFT_HOPN_PORT=0 so the control plane renders no Tor inbound/route on this device.
cat > "$SK/node.env" <<ENV
WEFT_HOST=${RELAY}
WEFT_REALITY_PBK=${PUB}
WEFT_REALITY_PRIV=${PRIV}
WEFT_SID=${SID}
WEFT_SNI=${SNI}
WEFT_HOP1_PORT=${LOCAL_HOP1}
WEFT_HOPN_PORT=0
WEFT_PUBLIC_HOP1_PORT=${PORT}
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
  sudo systemctl enable weft-node-cp weft-node-frpc weft-node-xray >/dev/null 2>&1 || true
  # restart (not just --now) so a re-run actually re-applies config changes (SNI, port, keys).
  sudo systemctl restart weft-node-cp     # writes xray.json + restarts xray
  sudo systemctl restart weft-node-frpc
else
  LA="$HOME/Library/LaunchAgents"; mkdir -p "$LA"
  # unload-then-load so a re-run actually restarts the service with the new config (SNI, port, keys).
  plist() { local p="$LA/com.weft.node.$1.plist"; cat > "$p"; launchctl unload "$p" 2>/dev/null || true; launchctl load -w "$p" 2>/dev/null || true; }
  printf '<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>com.weft.node.xray</string><key>ProgramArguments</key><array><string>%s</string><string>run</string><string>-c</string><string>%s/xray.json</string></array><key>KeepAlive</key><true/><key>RunAtLoad</key><true/></dict></plist>' "$XRAY" "$SK" | plist xray
  printf '<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>com.weft.node.frpc</string><key>ProgramArguments</key><array><string>%s/frpc</string><string>-c</string><string>%s/frpc.toml</string></array><key>KeepAlive</key><true/><key>RunAtLoad</key><true/></dict></plist>' "$SK" "$SK" | plist frpc
  printf '<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>com.weft.node.cp</string><key>ProgramArguments</key><array><string>%s</string><string>%s/control-plane.mjs</string></array><key>EnvironmentVariables</key><dict><key>WEFT_ENVFILE</key><string>%s/node.env</string></dict><key>KeepAlive</key><true/><key>RunAtLoad</key><true/></dict></plist>' "$NODE" "$SK" "$SK" | plist cp
fi

cat <<DONE

✅ Weft 1-hop node is up (persistent — survives reboot, crash, closing the terminal).
   Public endpoint:  ${RELAY}:${PORT}   ·   region: ${GEO}

It was registered + paid for on the website, so this never touches the chain — restarting is free.
Your node is LIVE and earning \$WEFT for the traffic it carries — see it in the cabinet → network.
Stop being a node any time:  ./scripts/stop-node.sh   (no key needed; restart with run-node.sh).
DONE
