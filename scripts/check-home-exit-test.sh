#!/usr/bin/env bash
#
# Local verifier for scripts/test-home-exit.sh.
# It connects to the public relay endpoint as a client, exposes a temporary SOCKS port,
# and compares direct vs proxied public IPs.
set -euo pipefail

WORK="${WEFT_TEST_DIR:-$HOME/.weft-home-exit-test}"
RELAY="${WEFT_RELAY:-vpn.weftnetwork.net}"
SNI="${WEFT_SNI:-ya.ru}"
SOCKS_PORT="${WEFT_TEST_SOCKS_PORT:-10888}"

if [ ! -f "$WORK/identity.env" ]; then
  echo "missing $WORK/identity.env; run ./scripts/test-home-exit.sh first"
  exit 1
fi

# shellcheck disable=SC1091
. "$WORK/identity.env"
PORT="${WEFT_TEST_PORT:-$PORT}"

if [ "$(uname -s)" = "Darwin" ]; then
  XRAY="${WEFT_TEST_XRAY:-$WORK/xray-1.8.24}"
else
  XRAY="$(command -v xray)"
fi

cat > "$WORK/client.json" <<JSON
{
  "log": { "loglevel": "debug" },
  "inbounds": [
    {
      "listen": "127.0.0.1",
      "port": ${SOCKS_PORT},
      "protocol": "socks",
      "settings": { "auth": "noauth", "udp": false }
    }
  ],
  "outbounds": [
    {
      "protocol": "vless",
      "settings": {
        "vnext": [
          {
            "address": "${RELAY}",
            "port": ${PORT},
            "users": [
              { "id": "${UUID}", "encryption": "none", "flow": "xtls-rprx-vision" }
            ]
          }
        ]
      },
      "streamSettings": {
        "network": "tcp",
        "security": "reality",
        "realitySettings": {
          "fingerprint": "firefox",
          "serverName": "${SNI}",
          "publicKey": "${PUB}",
          "shortId": "${SID}",
          "spiderX": "/"
        }
      }
    }
  ]
}
JSON

rm -f "$WORK/client.log" "$WORK/client.pid"
"$XRAY" run -c "$WORK/client.json" > "$WORK/client.log" 2>&1 &
echo $! > "$WORK/client.pid"

cleanup() {
  kill "$(cat "$WORK/client.pid" 2>/dev/null)" 2>/dev/null || true
  rm -f "$WORK/client.pid"
}
trap cleanup EXIT

sleep 2
if ! kill -0 "$(cat "$WORK/client.pid")" 2>/dev/null; then
  echo "client xray failed to start"
  sed -n '1,160p' "$WORK/client.log"
  exit 1
fi

DIRECT_IP="$(curl -fsS --max-time 10 https://api.ipify.org)"
PROXIED_IP="$(curl --socks5-hostname "127.0.0.1:${SOCKS_PORT}" -fsS --max-time 20 https://api.ipify.org)"

cat <<DONE
direct_ip=${DIRECT_IP}
proxied_ip=${PROXIED_IP}
DONE
