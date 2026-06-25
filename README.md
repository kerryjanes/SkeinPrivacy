# Weft

**A decentralized VPN on Solana.** Weft is a DePIN network: anyone runs a node, carries other
people's traffic, and earns `$WEFT`. Each node offers **two connection modes**:

- **1-hop** — direct VLESS + Reality exit. Fast; traffic egresses at the node.
- **multihop** — VLESS + Reality, then routed through the **Tor network**. Onion routing, maximum
  privacy (the exit never learns who you are); slower, egresses at a Tor exit.

- **Data plane:** battle-tested **Xray-core (VLESS + Reality)** for DPI-resistant transport +
  the **Tor** daemon for the multihop onion path. No custom protocol — Weft's value-add is the
  polished node + the on-chain incentive/registry layer.
- **Token:** `$WEFT` (SPL), used to reward node operators, stake on nodes, and vote in governance.
- **Registry:** every node is a compressed NFT on Solana, carrying its geo, capabilities, stake and reputation.

The protocol speaks **VLESS + Reality**, so you connect with apps you already trust — **V2Box,
Happ, sing-box, Hiddify, Streisand** — on any OS. Reality masquerades the connection as ordinary
HTTPS to a real site, so DPI (incl. Russia's TSPU) can't tell it from normal traffic.

---

## Use Weft as a VPN

You don't need to run a node to use the network.

### 1. With an app you already use (recommended)

Weft works with the popular VLESS clients — on phone **and** computer:
**V2Box**, **Happ**, **sing-box**, **Hiddify**, **Streisand** (iOS · Android · macOS · Windows · Linux).

1. Install any one of them.
2. Get a `vless://` link for a node — its **1-hop** (fast) or **multihop** (Tor, max privacy) link.
   (Running your own node? `deploy-node.sh` prints both.)
3. Import the link into the client and press **Connect**.

### 2. The Weft app

Download the app from [**Releases**](https://github.com/kerryjanes/WeftNetwork/releases),
open it, and press **Connect**. Nothing to paste.

---

## Run a node

A Weft node is a VPS that serves both connection modes and earns `$WEFT`. Rewards are weighted
by reputation (0.5×–2.0×), geo demand (up to +50%) and stake (+20% at 10,000 `$WEFT`).

### With one script (recommended)

```sh
sudo ./scripts/deploy-node.sh            # or: sudo ./scripts/deploy-node.sh ya.ru   (DPI-heavy markets)
```

On a fresh Ubuntu/Debian VPS the script installs **Xray-core** (VLESS + Reality) and the **Tor**
daemon, configures both modes (1-hop direct + multihop via Tor), and prints two ready connection
links (+ a QR) to paste into Happ / V2Box / any VLESS client. The data plane is standard,
battle-tested Xray + Tor — nothing to maintain by hand.

---

## Repository layout

| Path | What |
| --- | --- |
| `programs/` | 7 Anchor programs: token vesting, node registry (cNFT), staking, reputation, rewards settlement, governance (DAO), IDO/token distributor. |
| `crates/weft-primitives` | Shared tokenomics + reward/split/merkle math (single source of truth, on- and off-chain). |
| `scripts/deploy-node.sh` | One-command node setup: Xray (VLESS+Reality) + Tor, both connection modes, prints the links. |
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
