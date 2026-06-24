# Weft

**A decentralized VPN on Solana.** Weft is a DePIN network: traffic is onion-routed over the
**Tor network** — multi-hop, so every relay only knows the next one and the exit never learns
who you are — and `$WEFT` rewards people for running relays. It's incentivized Tor: the
strongest, most battle-tested anonymity network as the data plane, with an on-chain incentive
and registry layer on top.

- **Token:** `$WEFT` (SPL), used to reward relay operators, stake on nodes, and vote in governance.
- **Registry:** every node is a compressed NFT on Solana, carrying its geo, capabilities, stake and reputation.
- **Data plane:** the Tor network, embedded via [Arti](https://gitlab.torproject.org/tpo/core/arti) (the
  pure-Rust Tor client) — 20 years of hardened multi-hop onion routing, flow control, and NAT traversal.

The protocol speaks **VLESS**, so you can connect with apps you already trust — **V2Box, Happ,
sing-box, Hiddify** — on any OS, or with the Weft desktop app.

---

## Use Weft as a VPN

You don't need to run a node to use the network.

### 1. With an app you already use (recommended)

Weft works with the popular VLESS clients — on phone **and** computer:
**V2Box**, **Happ**, **sing-box**, **Hiddify** (iOS · Android · macOS · Windows · Linux).

1. Install any one of them.
2. Get a connection link (from the Weft app, or run `./scripts/share-link.sh` to use your own node).
3. Import the link into the client and press **Connect**.

### 2. The Weft app

Download the app from [**Releases**](https://github.com/kerryjanes/WeftNetwork/releases),
open it, and press **Connect**. Nothing to paste.

---

## Run a node

A Weft node is a **Tor relay**: run one, carry traffic for the network Weft routes over, and
earn `$WEFT`. Rewards are weighted by reputation (0.5×–2.0×), geo demand (up to +50%) and
stake (+20% at 10,000 `$WEFT`).

### With one script (recommended)

```sh
./scripts/run-relay.sh
```

The script installs Tor, configures a **non-exit relay** (safe to run from home — it never
sends traffic to the open internet on your behalf), creates your operator key, registers the
relay on-chain, and starts it. Re-run it any time. To run an exit relay instead (extra demand,
extra reward, but you egress others' traffic), pass `--exit` and read the printed warning first.

---

## Repository layout

| Path | What |
| --- | --- |
| `programs/` | 7 Anchor programs: token vesting, node registry (cNFT), staking, reputation, rewards settlement, governance (DAO), IDO/token distributor. |
| `crates/weft-primitives` | Shared tokenomics + reward/split/merkle math (single source of truth, on- and off-chain). |
| `crates/weft-vpn` | The data plane: routes traffic through Tor (Arti) behind a VLESS gateway + SOCKS5 front-end (`weft-vpn` CLI). |
| `services/` | TypeScript services + CLIs: registry provisioning, the settlement aggregator, the node-directory indexer, governance tooling, and genesis. |
| `sdk/` | `@weft/sdk` — typed Solana clients for every program, plus the shared math. |
| `clients/desktop/` | The cross-platform desktop VPN app. |

On-chain program IDs (Solana devnet):

```
node-registry        6dsqVjMmczosqNk2kaFHa33ut9ZUAwazgUagPKk5tUgd
staking              86FwTDBau7T289G9Fnkjn34g7NN3furoGEDwFsLVXzTK
reputation           6Nwa73bqP56LNQwWEKJWAp4A5RJKSMBzFxdxtuq3Y86u
rewards-settlement   BMQZKvCbq8qcZFWWGt1S7ZXg8odZcMbUA3oaNKnQi7mz
governance           q3K9krqiQDL7WHVUzLZrjJLgsM53vSrcfNRTzsVE6eA
token-distributor    2vXouVhktgRQhBhuvUMpi6hCryEtAsJeHJMKG9QgxpzV
vesting              FCFZNb2Kqh7ScjikKp73W7BcsfusrZ1hTBhc61Macdsv
```

## Build from source

Requires Rust (stable), Node 22+ with pnpm, and the Anchor/Solana toolchain for the programs.

```sh
pnpm install                 # JS workspaces (services, sdk, desktop)
cargo build --release        # host crates (net / node / vpn / primitives)
cargo test --workspace       # unit + integration tests
anchor build                 # the on-chain programs
```

## Contributing

Issues and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Please report
security issues privately as described in [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE).
