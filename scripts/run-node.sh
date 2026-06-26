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
#   First run:   ./weft-node.sh <your-node-key>     # the key copied from cabinet -> deploy
#   After that:  ./weft-node.sh                     # the key is saved; no need to pass it
#   Stop:        ./weft-node.sh stop                # stop being a node (no key needed)
#   Purge:       ./weft-node.sh stop --purge        # remove local services + ~/.weft
#
# The node key already contains the identity, region, and relay port — there is nothing else to type.
# Registration + payment happened on the website; this script never touches the chain.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SK="$HOME/.weft"
mkdir -p "$SK"

stop_node() {
  local purge="${1:-}"
  local os
  os=$(uname -s)
  echo "→ stopping Weft node…"
  if [ "$os" = "Linux" ]; then
    for unit_name in weft-node-cp weft-node-frpc weft-node-xray; do
      sudo systemctl disable --now "$unit_name" 2>/dev/null || true
      if [ "$purge" = "--purge" ]; then sudo rm -f "/etc/systemd/system/$unit_name.service"; fi
    done
    sudo systemctl daemon-reload 2>/dev/null || true
  else
    local launch_agents="$HOME/Library/LaunchAgents"
    for label in xray frpc cp; do
      launchctl unload -w "$launch_agents/com.weft.node.$label.plist" 2>/dev/null || true
      if [ "$purge" = "--purge" ]; then rm -f "$launch_agents/com.weft.node.$label.plist"; fi
    done
  fi

  if [ "$purge" = "--purge" ]; then
    rm -rf "$SK"
    echo "✅ node stopped + purged (~/.weft and services removed)."
  else
    echo "✅ node stopped. Your keys/config remain in ~/.weft — run ./weft-node.sh to start again."
    echo "   (Your on-chain registration stays; the node is just offline until restarted.)"
  fi
}

case "${1:-}" in
  stop|--stop)
    stop_node "${2:-}"
    exit 0
    ;;
esac

TOKEN="${1:-${WEFT_NODE_KEY:-}}"

OS=$(uname -s)
echo "→ dependencies…"
if [ "$OS" = "Linux" ]; then
  sudo apt-get update -y >/dev/null
  DEBIAN_FRONTEND=noninteractive sudo apt-get install -y curl openssl python3 iproute2 >/dev/null
  command -v node >/dev/null 2>&1 || { curl -fsSL https://deb.nodesource.com/setup_20.x | sudo bash - >/dev/null 2>&1; sudo apt-get install -y nodejs >/dev/null 2>&1; }
  command -v xray >/dev/null 2>&1 || sudo bash -c "$(curl -L -s https://github.com/XTLS/Xray-install/raw/main/install-release.sh)" @ install --version 1.8.24 >/dev/null
  XRAY=/usr/local/bin/xray; ARCH=linux_amd64; NODE=$(command -v node)
else
  echo "  (macOS: ensure node is installed via brew: brew install node)"
  command -v node >/dev/null 2>&1 || { echo "missing dependency: node"; exit 1; }
  command -v unzip >/dev/null 2>&1 || { echo "missing dependency: unzip"; exit 1; }
  ARCH=darwin_$( [ "$(uname -m)" = "arm64" ] && echo arm64 || echo amd64 ); NODE=$(command -v node)
  XRAY="$SK/xray-1.8.24"
  if [ ! -x "$XRAY" ]; then
    case "$(uname -m)" in
      arm64) XRAY_ZIP="Xray-macos-arm64-v8a.zip" ;;
      *) XRAY_ZIP="Xray-macos-64.zip" ;;
    esac
    echo "→ xray-core 1.8.24 (stable Reality/Vision)…"
    curl -fsSL "https://github.com/XTLS/Xray-core/releases/download/v1.8.24/${XRAY_ZIP}" -o "$SK/xray.zip"
    rm -rf "$SK/xray-1.8.24.extract"
    mkdir -p "$SK/xray-1.8.24.extract"
    unzip -oq "$SK/xray.zip" -d "$SK/xray-1.8.24.extract"
    mv "$SK/xray-1.8.24.extract/xray" "$XRAY"
    chmod +x "$XRAY"
    rm -rf "$SK/xray.zip" "$SK/xray-1.8.24.extract"
  fi
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
CLUSTER="${WEFT_CLUSTER:-devnet}"
RPC="${WEFT_RPC:-https://api.devnet.solana.com}"
MINT="${WEFT_MINT:-Hfvwx9F5NDzMCyywJZJsFVX83XaXnLNntCdk21h7Bmcy}"
if [[ "$CLUSTER" == mainnet* ]]; then
  [ -n "${WEFT_RPC:-}" ] || { echo "WEFT_RPC must be set explicitly for ${CLUSTER}"; exit 1; }
  [ -n "${WEFT_MINT:-}" ] || { echo "WEFT_MINT must be set explicitly for ${CLUSTER}"; exit 1; }
fi
LOCAL_HOP1=14430
FRP_VER="0.69.1"
RAW="${WEFT_RAW_BASE:-https://raw.githubusercontent.com/kerryjanes/WeftNetwork/rehearsal/devnet-mainnet-flow}"

active_ipv4_for_iface() {
  local iface="$1"
  if [ "$OS" = "Darwin" ]; then
    ifconfig "$iface" 2>/dev/null | grep -q "status: active" || return 1
    ipconfig getifaddr "$iface" 2>/dev/null || return 1
  else
    ip -o -4 addr show dev "$iface" scope global 2>/dev/null | awk 'NR == 1 { sub("/.*", "", $4); print $4 }'
  fi
}

detect_physical_egress() {
  local iface ip_addr
  if [ "$OS" = "Darwin" ]; then
    for iface in $(ifconfig -l | tr ' ' '\n' | grep -E '^en[0-9]+$'); do
      ip_addr="$(active_ipv4_for_iface "$iface" || true)"
      if [ -n "$ip_addr" ]; then
        printf '%s %s\n' "$iface" "$ip_addr"
        return 0
      fi
    done
  else
    while read -r iface _; do
      case "$iface" in
        lo|tun*|tap*|utun*|wg*|tailscale*|docker*|br-*|veth*) continue ;;
      esac
      ip_addr="$(active_ipv4_for_iface "$iface" || true)"
      if [ -n "$ip_addr" ]; then
        printf '%s %s\n' "$iface" "$ip_addr"
        return 0
      fi
    done < <(ip -o link show up | awk -F': ' '{print $2}')
  fi
  return 1
}

EGRESS_INTERFACE="${WEFT_EGRESS_INTERFACE:-auto}"
SEND_THROUGH="${WEFT_XRAY_SEND_THROUGH:-}"
if [ -z "$SEND_THROUGH" ] && [ "$EGRESS_INTERFACE" != "none" ]; then
  if [ "$EGRESS_INTERFACE" = "auto" ]; then
    DETECTED="$(detect_physical_egress || true)"
    EGRESS_INTERFACE="${DETECTED%% *}"
    SEND_THROUGH="${DETECTED#* }"
    if [ "$EGRESS_INTERFACE" = "$SEND_THROUGH" ]; then SEND_THROUGH=""; fi
  else
    SEND_THROUGH="$(active_ipv4_for_iface "$EGRESS_INTERFACE" || true)"
  fi
fi
if [ -n "$SEND_THROUGH" ]; then
  echo "→ 1-hop egress pinned to ${EGRESS_INTERFACE} (${SEND_THROUGH}) to bypass host VPN routes"
else
  echo "→ 1-hop egress uses the system default route"
fi

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

if [ -f "$REPO_ROOT/services/control-plane/dist/control-plane.mjs" ]; then
  cp "$REPO_ROOT/services/control-plane/dist/control-plane.mjs" "$SK/control-plane.mjs"
else
  curl -fsSL "${RAW}/services/control-plane/dist/control-plane.mjs" -o "$SK/control-plane.mjs"
fi

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
WEFT_XRAY_SEND_THROUGH=${SEND_THROUGH}
WEFT_STORE=${SK}/users.json
WEFT_PORT=8088
WEFT_CLUSTER=${CLUSTER}
WEFT_RPC=${RPC}
WEFT_MINT=${MINT}
WEFT_RELAY_TOKEN=${RELAY_TOKEN}
WEFT_RELAY_PROFILE_URL=https://${RELAY}:8089/relay/node-profile
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
Stop being a node any time:  ./weft-node.sh stop   (no key needed; restart with ./weft-node.sh).
DONE
