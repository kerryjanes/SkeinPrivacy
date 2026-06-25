# @weft/control-plane

The per-node service that makes Weft access **token-gated**. It runs on each node (installed by
`scripts/run-node.sh` / `deploy-node.sh`) and:

- **Mints a personal `vless://` link per wallet** (`POST /provision {wallet}`) — its own UUID,
  metered to that wallet.
- **Meters** each user's traffic via Xray's stats API and **enforces the `$WEFT` budget**: a user
  stays connected only while their consumption is within what their balance pays for
  (`unsettledBytes < balance/price`, price = 0.1 `$WEFT`/GB). Over budget → the user is removed
  from the Xray config (link stops); top up → restored.
- **Owns the Xray config**: renders `/usr/local/etc/xray/config.json` (the two Reality inbounds +
  api/stats) from the active user set and reloads Xray when it changes.
- **Tracks per-node served traffic** (`GET /node/stats`) — the basis for the operator's `$WEFT`
  earnings (`trafficReward` over served bytes).
- Verifies a user's on-chain `pay_traffic` settlement (`POST /settle {wallet, signature}`).

## HTTP API

| Route                                 | What                                                       |
| ------------------------------------- | ---------------------------------------------------------- |
| `POST /provision {wallet}`            | mint/refresh the wallet's personal links + budget status   |
| `GET /status?wallet=…`                | current quota / used / remaining (same shape as provision) |
| `POST /settle {wallet, signature}`    | register a verified `pay_traffic` payment, clear the tab   |
| `GET /node/stats`                     | this node's total served bytes → `$WEFT` earned           |
| `GET /price`                          | price/GB, mint, host, modes, faucet availability           |
| `POST /faucet {wallet}` (devnet only) | mint test `$WEFT` to a wallet (test-mint nodes only)      |

## Config (env, with launch-node defaults)

`WEFT_HOST` · `WEFT_REALITY_PBK/PRIV` · `WEFT_SID` · `WEFT_SNI` · `WEFT_HOP1_PORT` /
`WEFT_HOPN_PORT` (local Xray listen) · `WEFT_PUBLIC_HOP1_PORT` / `WEFT_PUBLIC_HOPN_PORT` (relay
ports advertised in links — for home nodes behind a relay) · `WEFT_FOUNDER_UUID` · `WEFT_PORT`
(default 8088) · `WEFT_RPC` / `WEFT_WS` · `WEFT_MINT` · `WEFT_XRAY_RELOAD` (how to reload Xray)
· `WEFT_FAUCET_KEYPAIR` (enables the devnet faucet).

A public deployment fronts it with TLS (e.g. Caddy) so the browser cabinet can call it over HTTPS.

```sh
pnpm --filter @weft/control-plane test      # unit tests
pnpm --filter @weft/control-plane bundle    # → dist/control-plane.mjs (what run-node.sh fetches)
```
