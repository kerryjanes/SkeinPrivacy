# Weft

**A decentralized VPN on Solana.** Weft is a DePIN network: anyone can run a node, route
other people's encrypted traffic, and get paid in `$WEFT` per gigabyte. Traffic is onion-routed
through 3–5 nodes — every hop only knows the next one, and the exit never learns who you are.

- **Token:** `$WEFT` (SPL), used to pay for traffic, stake on nodes, and vote in governance.
- **Pricing:** `0.1 $WEFT` per GB. Each payment splits **70% to nodes / 20% burned / 10% treasury**.
- **Registry:** every node is a compressed NFT on Solana, carrying its geo, capabilities, stake and reputation.
- **Transport:** WireGuard-style link encryption + a single-pass Sphinx onion over a libp2p/Kademlia network.

The protocol speaks **VLESS**, so you can connect with apps you already trust — **V2Box, Happ,
sing-box, Hiddify** — on any OS, or with the Weft desktop app.

---

## Use Weft as a VPN

You don't need to run a node to use the network. Pick whichever fits you.

### 1. Any VLESS client (phone, desktop — any OS)

1. Install a VLESS client: **V2Box**, **Happ**, **sing-box**, or **Hiddify** (iOS · Android · macOS · Windows · Linux).
2. Get a `vless://` link from a node operator (or from your own node — see below).
3. Paste it into the client and connect. Your traffic now exits the Weft network.

### 2. Desktop app (macOS / Windows / Linux)

Download the latest build from [**Releases**](https://github.com/kerryjanes/WeftNetwork/releases),
open it, and click **Connect**. It bundles the engine and routes everything through Weft —
proxy mode (no admin) or full-tunnel. No link to paste.

### 3. From the command line

Run a local SOCKS5 proxy backed by a self-contained circuit, then point any app at it:

```sh
cargo run -p weft-vpn --release -- socks 127.0.0.1:1080
curl --proxy socks5h://127.0.0.1:1080 https://example.com
```

Or expose a VLESS gateway (what V2Box/Happ dial into):

```sh
cargo run -p weft-vpn --release -- vless 0.0.0.0:8443 --tls   # prints a vless:// link
```

To route through a real, multi-node network instead of a local circuit, set `WEFT_PEERS` to a
node directory (see `services/indexer`).

---

## Run a node

Run a node, route traffic, and earn `$WEFT`.

A node registers itself on-chain as a compressed NFT, joins the DHT, and relays/exits traffic.
The operator's key is **both** its on-chain identity and the daemon's identity — one keypair.

### Via the CLI / SDK

```sh
# 1. Build the daemon
cargo build -p weft-node --release

# 2. Generate (or reuse) the operator key and start the node
export WEFT_OPERATOR_KEY=$(openssl rand -hex 96)     # operator‖static‖onion seeds
target/release/weft-node                              # joins the DHT, starts relaying

# 3. Register on-chain (mints your node's cNFT in the registry)
export WEFT_RPC_URL=<a DAS-capable Solana RPC>        # cNFTs require a DAS RPC (e.g. Helius)
pnpm --filter @weft/registry-provision node:register
pnpm --filter @weft/registry-provision node:status    # verify it's live
```

Rewards settle every epoch (~10 min): the off-chain aggregator posts a Merkle root on-chain and
you `claim` your `$WEFT`, weighted by traffic, reputation (0.5×–2.0×), geo demand (up to +50%) and
stake (+20% at 10,000 `$WEFT`). See `services/aggregator` and `@weft/sdk`.

### Via the desktop app

The desktop app can register your machine as a node and run it for you — no terminal required.

---

## Repository layout

| Path | What |
| --- | --- |
| `programs/` | 7 Anchor programs: token vesting, node registry (cNFT), staking, reputation, rewards settlement, governance (DAO), IDO/token distributor. |
| `crates/weft-primitives` | Shared tokenomics + reward/split/merkle math (single source of truth, on- and off-chain). |
| `crates/weft-net` | Onion data plane: Sphinx over Ristretto, WireGuard-style links, Kademlia discovery, circuit selection, metering. |
| `crates/weft-node` | The node daemon (relay + exit). |
| `crates/weft-vpn` | The VLESS gateway, SOCKS5 front-end, real-egress exit, and `weft-vpn` CLI. |
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
