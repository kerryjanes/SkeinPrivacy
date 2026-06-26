#!/usr/bin/env bash
#
# Temporary E2E test for the core Weft data-plane claim:
#
#   phone -> public relay VPS -> this computer -> internet
#
# The phone should see this computer's public IP, not the relay/VPS IP.
# This script does not touch Solana, the cabinet, node registration, staking, or rewards.
set -euo pipefail

WORK="${WEFT_TEST_DIR:-$HOME/.weft-home-exit-test}"
RELAY="${WEFT_RELAY:-vpn.weftnetwork.net}"
RELAY_PORT="${WEFT_RELAY_PORT:-7000}"
RELAY_TOKEN="${WEFT_RELAY_TOKEN:-a40b1ab498a37ba6bbaa70791ac62287}"
SNI="${WEFT_SNI:-ya.ru}"
# Keep this separate from the normal home-node port (14430) so the E2E test can run
# while a development node is already installed/running on this machine.
LOCAL_HOP1="${WEFT_TEST_LOCAL_PORT:-15430}"
FRP_VER="${WEFT_FRP_VERSION:-0.69.1}"
BYPASS_INTERFACE="${WEFT_TEST_BYPASS_INTERFACE:-}"
SEND_THROUGH="${WEFT_TEST_SEND_THROUGH:-}"

mkdir -p "$WORK"

OS="$(uname -s)"
ARCH=""
case "$OS" in
  Linux) ARCH="linux_amd64" ;;
  Darwin)
    if [ "$(uname -m)" = "arm64" ]; then ARCH="darwin_arm64"; else ARCH="darwin_amd64"; fi
    ;;
  *) echo "unsupported OS: $OS"; exit 1 ;;
esac

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing dependency: $1"
    return 1
  }
}

if [ "$OS" = "Linux" ] && ! command -v xray >/dev/null 2>&1; then
  if [ "$OS" = "Linux" ]; then
    echo "→ installing xray-core..."
    sudo bash -c "$(curl -L -s https://github.com/XTLS/Xray-install/raw/main/install-release.sh)" @ install --version 1.8.24 >/dev/null
  fi
fi
need curl
need openssl
need python3

if [ "$OS" = "Darwin" ]; then
  XRAY="${WEFT_TEST_XRAY:-$WORK/xray-1.8.24}"
  if [ ! -x "$XRAY" ]; then
    need unzip
    case "$(uname -m)" in
      arm64) XRAY_ZIP="Xray-macos-arm64-v8a.zip" ;;
      *) XRAY_ZIP="Xray-macos-64.zip" ;;
    esac
    echo "→ downloading xray-core 1.8.24..."
    curl -fsSL "https://github.com/XTLS/Xray-core/releases/download/v1.8.24/${XRAY_ZIP}" -o "$WORK/xray.zip"
    rm -rf "$WORK/xray-1.8.24.extract"
    mkdir -p "$WORK/xray-1.8.24.extract"
    unzip -oq "$WORK/xray.zip" -d "$WORK/xray-1.8.24.extract"
    mv "$WORK/xray-1.8.24.extract/xray" "$XRAY"
    chmod +x "$XRAY"
    rm -rf "$WORK/xray.zip" "$WORK/xray-1.8.24.extract"
  fi
else
  XRAY="$(command -v xray)"
fi

if [ ! -x "$WORK/frpc" ]; then
  echo "→ downloading frpc..."
  curl -fsSL "https://github.com/fatedier/frp/releases/download/v${FRP_VER}/frp_${FRP_VER}_${ARCH}.tar.gz" -o "$WORK/frp.tgz"
  tar xzf "$WORK/frp.tgz" -C "$WORK"
  cp "$WORK/frp_${FRP_VER}_${ARCH}/frpc" "$WORK/frpc"
  chmod +x "$WORK/frpc"
  rm -rf "$WORK/frp.tgz" "$WORK/frp_${FRP_VER}_${ARCH}"
fi

if [ ! -f "$WORK/identity.env" ]; then
  echo "→ generating temporary Reality identity..."
  KEYS="$("$XRAY" x25519)"
  PRIV="$(printf '%s\n' "$KEYS" | awk 'tolower($0) ~ /private/ {print $NF; exit}')"
  PUB="$(printf '%s\n' "$KEYS" | awk 'tolower($0) ~ /public|password/ {print $NF; exit}')"
  UUID="$("$XRAY" uuid)"
  SID="$(openssl rand -hex 8)"
  PORT="$(
    python3 - "$PUB" <<'PY'
import hashlib, sys
pub = sys.argv[1].encode()
# Keep inside the same small relay range the existing home-node scripts use.
print(20000 + (int(hashlib.sha256(pub).hexdigest(), 16) % 80))
PY
  )"
  cat > "$WORK/identity.env" <<ENV
PRIV=${PRIV}
PUB=${PUB}
UUID=${UUID}
SID=${SID}
PORT=${PORT}
ENV
fi

# shellcheck disable=SC1091
. "$WORK/identity.env"
PORT="${WEFT_TEST_PORT:-$PORT}"

if [ -z "$SEND_THROUGH" ] && [ -n "$BYPASS_INTERFACE" ]; then
  if command -v ipconfig >/dev/null 2>&1; then
    SEND_THROUGH="$(ipconfig getifaddr "$BYPASS_INTERFACE" 2>/dev/null || true)"
  fi
fi
OUTBOUND_SEND_THROUGH_JSON=""
if [ -n "$SEND_THROUGH" ]; then
  OUTBOUND_SEND_THROUGH_JSON="\"sendThrough\": \"${SEND_THROUGH}\","
fi

rm -f "$WORK/access.log" "$WORK/error.log" "$WORK/xray.log" "$WORK/frpc.log"

cat > "$WORK/xray.json" <<JSON
{
  "log": {
    "loglevel": "debug",
    "access": "${WORK}/access.log",
    "error": "${WORK}/error.log"
  },
  "inbounds": [
    {
      "tag": "hop1",
      "listen": "127.0.0.1",
      "port": ${LOCAL_HOP1},
      "protocol": "vless",
      "settings": {
        "clients": [
          { "id": "${UUID}", "flow": "xtls-rprx-vision" }
        ],
        "decryption": "none"
      },
      "streamSettings": {
        "network": "tcp",
        "security": "reality",
        "realitySettings": {
          "show": false,
          "dest": "${SNI}:443",
          "xver": 0,
          "serverNames": ["${SNI}"],
          "privateKey": "${PRIV}",
          "shortIds": ["${SID}"]
        }
      }
    }
  ],
  "outbounds": [
    {
      "tag": "direct",
      ${OUTBOUND_SEND_THROUGH_JSON}
      "protocol": "freedom",
      "settings": { "domainStrategy": "UseIPv4" }
    }
  ]
}
JSON

cat > "$WORK/frpc.toml" <<TOML
serverAddr = "${RELAY}"
serverPort = ${RELAY_PORT}
auth.method = "token"
auth.token = "${RELAY_TOKEN}"

[[proxies]]
name = "weft-home-exit-test-${UUID:0:8}"
type = "tcp"
localIP = "127.0.0.1"
localPort = ${LOCAL_HOP1}
remotePort = ${PORT}
TOML

"$(dirname "$0")/stop-home-exit-test.sh" >/dev/null 2>&1 || true

echo "→ starting local xray..."
if [ "$OS" = "Darwin" ] && command -v launchctl >/dev/null 2>&1; then
  UID_VALUE="$(id -u)"
  XRAY_LABEL="com.weft.home-exit-test.xray"
  cat > "$WORK/xray.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>${XRAY_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${XRAY}</string>
    <string>run</string>
    <string>-c</string>
    <string>${WORK}/xray.json</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>StandardOutPath</key><string>${WORK}/xray.log</string>
  <key>StandardErrorPath</key><string>${WORK}/xray.stderr.log</string>
  </dict>
</plist>
PLIST
  launchctl bootstrap "gui/${UID_VALUE}" "$WORK/xray.plist"
  sleep 1
  launchctl print "gui/${UID_VALUE}/${XRAY_LABEL}" | awk '/pid = / { gsub(";", "", $3); print $3; exit }' > "$WORK/xray.pid"
else
  nohup "$XRAY" run -c "$WORK/xray.json" > "$WORK/xray.log" 2>&1 &
  echo $! > "$WORK/xray.pid"
fi

echo "→ opening relay tunnel ${RELAY}:${PORT} -> 127.0.0.1:${LOCAL_HOP1}..."
if [ "$OS" = "Darwin" ] && command -v launchctl >/dev/null 2>&1; then
  FRPC_LABEL="com.weft.home-exit-test.frpc"
  cat > "$WORK/frpc.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>${FRPC_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${WORK}/frpc</string>
    <string>-c</string>
    <string>${WORK}/frpc.toml</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>StandardOutPath</key><string>${WORK}/frpc.log</string>
  <key>StandardErrorPath</key><string>${WORK}/frpc.stderr.log</string>
</dict>
</plist>
PLIST
  launchctl bootstrap "gui/${UID_VALUE}" "$WORK/frpc.plist"
  sleep 1
  launchctl print "gui/${UID_VALUE}/${FRPC_LABEL}" | awk '/pid = / { gsub(";", "", $3); print $3; exit }' > "$WORK/frpc.pid"
else
  nohup "$WORK/frpc" -c "$WORK/frpc.toml" > "$WORK/frpc.log" 2>&1 &
  echo $! > "$WORK/frpc.pid"
fi

sleep 3

if ! kill -0 "$(cat "$WORK/xray.pid")" 2>/dev/null; then
  echo "xray failed to start. Log:"
  sed -n '1,120p' "$WORK/xray.log"
  exit 1
fi
if ! nc -z 127.0.0.1 "$LOCAL_HOP1" >/dev/null 2>&1; then
  echo "xray process started but local port 127.0.0.1:${LOCAL_HOP1} is not listening. Log:"
  sed -n '1,160p' "$WORK/xray.log"
  exit 1
fi
if ! kill -0 "$(cat "$WORK/frpc.pid")" 2>/dev/null; then
  echo "frpc failed to start. This usually means the relay port is already taken."
  echo "Try another port with: WEFT_TEST_PORT=20042 ./scripts/test-home-exit.sh"
  echo
  sed -n '1,160p' "$WORK/frpc.log"
  exit 1
fi

LINK="vless://${UUID}@${RELAY}:${PORT}?flow=xtls-rprx-vision&type=tcp&security=reality&fp=firefox&sni=${SNI}&pbk=${PUB}&sid=${SID}&spx=%2F#Weft-home-exit-test"
if [ -n "$BYPASS_INTERFACE" ]; then
  COMPUTER_IP="$(curl --interface "$BYPASS_INTERFACE" -fsS --max-time 8 https://api.ipify.org 2>/dev/null || true)"
else
  COMPUTER_IP="$(curl -fsS --max-time 8 https://api.ipify.org 2>/dev/null || true)"
fi

cat <<DONE

Temporary home-exit node is running.

Expected exit IP on the phone:
  ${COMPUTER_IP:-"(could not detect from this computer)"}

Xray outbound source:
  ${SEND_THROUGH:-"system default route"}

VLESS link:
${LINK}

How to test:
  1. Put the phone on mobile internet, not the same Wi-Fi.
  2. Import the VLESS link in Happ / V2Box / Hiddify / Streisand.
  3. Connect.
  4. Open https://ifconfig.me or https://ipinfo.io on the phone.
  5. The shown IP must match the expected IP above, not the VPS relay IP.

Logs:
  ${WORK}/xray.log
  ${WORK}/access.log
  ${WORK}/error.log
  ${WORK}/frpc.log

Stop test:
  ./scripts/stop-home-exit-test.sh

DONE

if command -v qrencode >/dev/null 2>&1; then
  echo "QR:"
  qrencode -t ansiutf8 "$LINK"
else
  echo "Optional: install qrencode to print a QR in terminal."
fi
