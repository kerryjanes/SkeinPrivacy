# Weft node — one image, all platforms Docker runs on. Bundles Xray-core + frpc + the
# control-plane so a node operator only runs:
#   docker run -d --restart unless-stopped -e WEFT_NODE_KEY=<key> weftnetwork/node
# No per-OS scripts, no runtime downloads. Build context is docker/ (not the repo root).
FROM node:22-slim

ARG XRAY_VER=1.8.24
ARG FRP_VER=0.69.1
ARG TARGETARCH
# Non-secret defaults, overridable at `docker run -e ...`. RELAY_TOKEN is baked at build (the public
# relay's token is not a secret, like a Tor relay). RPC defaults to the public mainnet endpoint.
ARG RELAY_TOKEN=SET_RELAY_TOKEN
ARG RPC_URL=https://api.mainnet-beta.solana.com
ARG MINT=SET_MINT

RUN apt-get update \
  && apt-get install -y --no-install-recommends curl unzip ca-certificates tini python3 procps \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/weft

# Xray-core + frpc for the target architecture (amd64 / arm64).
RUN set -eux; \
    case "$TARGETARCH" in \
      amd64) XZ=Xray-linux-64.zip;        FA=linux_amd64;; \
      arm64) XZ=Xray-linux-arm64-v8a.zip; FA=linux_arm64;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1;; \
    esac; \
    curl -fsSL "https://github.com/XTLS/Xray-core/releases/download/v${XRAY_VER}/${XZ}" -o /tmp/xray.zip; \
    unzip -oq /tmp/xray.zip -d /tmp/xray; install -m0755 /tmp/xray/xray /usr/local/bin/xray; \
    curl -fsSL "https://github.com/fatedier/frp/releases/download/v${FRP_VER}/frp_${FRP_VER}_${FA}.tar.gz" -o /tmp/frp.tgz; \
    tar xzf /tmp/frp.tgz -C /tmp; install -m0755 "/tmp/frp_${FRP_VER}_${FA}/frpc" /usr/local/bin/frpc; \
    rm -rf /tmp/xray* /tmp/frp*

COPY control-plane.mjs /opt/weft/control-plane.mjs
COPY node-entrypoint.sh /usr/local/bin/weft-node
COPY reload-xray.sh /usr/local/bin/weft-reload-xray
RUN chmod +x /usr/local/bin/weft-node /usr/local/bin/weft-reload-xray

ENV WEFT_RELAY=vpn.weftnetwork.net \
    WEFT_RELAY_PORT=7000 \
    WEFT_SNI=ya.ru \
    WEFT_CLUSTER=mainnet-beta \
    WEFT_RPC=${RPC_URL} \
    WEFT_MINT=${MINT} \
    WEFT_RELAY_TOKEN=${RELAY_TOKEN}

# tini as PID 1 for correct signal handling / zombie reaping.
ENTRYPOINT ["/usr/bin/tini","--","/usr/local/bin/weft-node"]
