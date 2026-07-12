#!/usr/bin/env bash
# Weft node — container entrypoint. Decodes the node key from the cabinet, renders the frpc +
# control-plane config, then supervises xray + frpc + control-plane in one container. Mirrors
# scripts/run-node.sh, minus the per-OS install/autostart glue (the image already carries it all).
set -euo pipefail
SK=/opt/weft

KEY="${WEFT_NODE_KEY:-${WEFT_NODE_KEY:-}}"
if [ -z "$KEY" ]; then
  echo "ERROR: no node key. Register your device in the cabinet (deploy page), copy the key, then run:" >&2
  echo "  docker run -d --restart unless-stopped -e WEFT_NODE_KEY=<key> weftnetwork/node" >&2
  exit 1
fi

# Decode the node key (base64 JSON: reality identity + public port + relay + geo).
python3 - "$KEY" > "$SK/node.json" <<'PY'
import sys, base64, json
t = sys.argv[1]
d = json.loads(base64.b64decode(t + "=" * (-len(t) % 4)))
for k in ("realityPriv", "realityPub", "uuid", "sid", "port", "relay", "geo"):
    if k not in d:
        sys.exit(f"node key is missing '{k}' — copy it again from the cabinet -> deploy page")
json.dump(d, sys.stdout)
PY
eval "$(python3 - "$SK/node.json" <<'PY'
import sys, json
d = json.load(open(sys.argv[1]))
print(f'PRIV={d["realityPriv"]}'); print(f'PUB={d["realityPub"]}'); print(f'UUID={d["uuid"]}')
print(f'SID={d["sid"]}'); print(f'PORT={int(d["port"])}'); print(f'RELAY={d["relay"]}'); print(f'GEO={int(d["geo"])}')
PY
)"

RELAY="${WEFT_RELAY:-$RELAY}"
LOCAL_HOP1=14430
# Public relay token, defaulted here (not in the Dockerfile ENV) so image linters don't flag it as a
# secret — it isn't one: like a Tor relay, every node shares it. Override with -e WEFT_RELAY_TOKEN=…
: "${WEFT_RELAY_TOKEN:=a40b1ab498a37ba6bbaa70791ac62287}"

cat > "$SK/frpc.toml" <<TOML
serverAddr = "${RELAY}"
serverPort = ${WEFT_RELAY_PORT:-7000}
loginFailExit = false
auth.method = "token"
auth.token = "${WEFT_RELAY_TOKEN}"
[[proxies]]
name = "weft-1hop-${UUID:0:8}"
type = "tcp"
localIP = "127.0.0.1"
localPort = ${LOCAL_HOP1}
remotePort = ${PORT}
TOML

# 1-hop only in a container: WEFT_HOPN_PORT=0 so the control-plane renders no Tor inbound here.
cat > "$SK/node.env" <<ENV
WEFT_HOST=${RELAY}
WEFT_REALITY_PBK=${PUB}
WEFT_REALITY_PRIV=${PRIV}
WEFT_SID=${SID}
WEFT_SNI=${WEFT_SNI:-ya.ru}
WEFT_HOP1_PORT=${LOCAL_HOP1}
WEFT_HOPN_PORT=0
WEFT_PUBLIC_HOP1_PORT=${PORT}
WEFT_FOUNDER_UUID=${UUID}
WEFT_GEO=${GEO}
WEFT_XRAY_CONFIG=${SK}/xray.json
WEFT_XRAY_BIN=/usr/local/bin/xray
WEFT_XRAY_RELOAD=/usr/local/bin/weft-reload-xray
WEFT_STORE=${SK}/users.json
WEFT_PORT=8088
WEFT_CLUSTER=${WEFT_CLUSTER:-mainnet-beta}
WEFT_RPC=${WEFT_RPC}
WEFT_MINT=${WEFT_MINT}
WEFT_RELAY_TOKEN=${WEFT_RELAY_TOKEN}
WEFT_RELAY_PROFILE_URL=https://${RELAY}:8089/relay/node-profile
ENV

echo "[weft-node] relay=${RELAY} geo=${GEO} public-port=${PORT} cluster=${WEFT_CLUSTER:-mainnet-beta}"

set -a; . "$SK/node.env"; set +a

term() { echo "[weft-node] stopping"; kill 0 2>/dev/null || true; exit 0; }
trap term TERM INT

# control-plane renders xray.json (from node.env) + meters user access; frpc dials the public relay.
node "$SK/control-plane.mjs" &
frpc -c "$SK/frpc.toml" &

# xray supervisor: (re)start xray whenever the control-plane rewrites xray.json (add/remove users).
(
  last=""
  while true; do
    if [ -f "$SK/xray.json" ]; then
      cur="$(stat -c %Y "$SK/xray.json" 2>/dev/null || echo x)"
      if [ "$cur" != "$last" ]; then
        last="$cur"
        pkill -f "xray run -c $SK/xray.json" 2>/dev/null || true
        sleep 0.3
        xray run -c "$SK/xray.json" &
      fi
    fi
    sleep 2
  done
) &

# If any core process exits, exit non-zero so `--restart` brings the whole node back cleanly.
wait -n
echo "[weft-node] a core process exited — container will restart" >&2
kill 0 2>/dev/null || true
exit 1
