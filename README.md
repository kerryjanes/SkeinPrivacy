# Weft

**A decentralized VPN on Solana.** Weft is a DePIN network: anyone runs a node, carries other
people's traffic, and earns `$WEFT`. Each node offers **two connection modes**:

- **1-hop** — direct VLESS + Reality exit. Fast; traffic egresses at the node.
- **multihop** — VLESS + Reality, then routed through the **Tor network**. Onion routing, maximum
  privacy (the exit never learns who you are); slower, egresses at a Tor exit.

- **Data plane:** battle-tested **Xray-core (VLESS + Reality)** for DPI-resistant transport +
  the **Tor** daemon for the multihop onion path. No custom protocol — Weft's value-add is the
  polished node + the on-chain incentive/registry layer.
- **Access is token-gated.** Every node runs a **control plane** that mints a _personal_ link per
  wallet, meters its traffic via Xray's stats API, and **cuts you off once you've used what your
  `$WEFT` pays for** (0.1 `$WEFT`/GB) — restoring you when you top up or earn `$WEFT` by running
  a node. No flat "one shared link, unlimited & free": your balance is your budget.
- **Token:** `$WEFT` (SPL), used to pay for traffic, reward node operators, stake on nodes, and vote.
- **Registry:** every node is a compressed NFT on Solana, carrying its geo, capabilities, stake and reputation.

The protocol speaks **VLESS + Reality**, so you connect with apps you already trust — **V2Box,
Happ, sing-box, Hiddify, Streisand** — on any OS. Reality masquerades the connection as ordinary
HTTPS to a real site, so DPI (incl. Russia's TSPU) can't tell it from normal traffic.

---

## Use Weft as a VPN

You don't need to run a node to use the network.

You need a Solana wallet holding `$WEFT` — that balance is your traffic budget (0.1 `$WEFT`/GB).

### 1. With an app you already use (recommended)

Weft works with the popular VLESS clients — on phone **and** computer:
**V2Box**, **Happ**, **sing-box**, **Hiddify**, **Streisand** (iOS · Android · macOS · Windows · Linux).

1. Install any one of them.
2. Open the **cabinet** (the web app at `/app`), connect your Solana wallet, and go to **VPN
   access** — it shows your `$WEFT` budget and your two **personal** links (`1-hop` = fast,
   `multihop` = Tor / max privacy), each metered to your wallet. Copy one. When you've used what
   your `$WEFT` covers the link stops working — top up your wallet (or earn `$WEFT` by running a
   node) and it comes back.

   _Advanced / scripted:_ the cabinet just calls a node's control plane —
   `curl -X POST https://<node>:8089/provision -d '{"wallet":"<YOUR_SOLANA_PUBKEY>"}'`.

3. Import the link into the client and press **Connect**.

### 2. The Weft app

Download the app from [**Releases**](https://github.com/kerryjanes/WeftNetwork/releases), load your
wallet, and press **Connect** — it provisions your personal link and shows your `$WEFT` budget
(balance, used, remaining) live. Nothing to paste.

---

## Run a node

A node serves both connection modes under **your wallet** and earns `$WEFT` **for the traffic it
actually carries** (metered at the node). Rewards are weighted by reputation (0.5×–2.0×), geo
demand (up to +50%) and stake (+20% at 10,000 `$WEFT`). Two ways to run one:

### A home device behind NAT (PC · router · always-on box)

```sh
WEFT_RELAY_TOKEN=<token> ./scripts/run-node.sh
```

A home device can't accept inbound connections, so the agent does what Tailscale-DERP /
Cloudflare-Tunnel / Tor relays do: it dials **out** to a public **rendezvous relay** which exposes
your node at a public `relay:port`. Users connect there; the relay forwards to your home Xray; your
traffic exits at home. The agent runs Xray (VLESS+Reality) + Tor + `frpc` (the tunnel) + the Weft
control plane (meters your served traffic, gates users by `$WEFT`), then prints your public
endpoint — register it on-chain in the cabinet to start earning.

### A public VPS (has its own IP)

```sh
sudo ./scripts/deploy-node.sh            # or: sudo ./scripts/deploy-node.sh ya.ru   (DPI-heavy markets)
```

On a fresh Ubuntu/Debian VPS the script installs **Xray-core** (VLESS + Reality), the **Tor**
daemon, and the control plane, serving both modes directly (no relay needed). The data plane is
standard, battle-tested Xray + Tor — nothing to maintain by hand.

---

## Repository layout

| Path                      | What                                                                                                                                               |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `programs/`               | 7 Anchor programs: token vesting, node registry (cNFT), staking, reputation, rewards settlement, governance (DAO), IDO/token distributor.          |
| `crates/weft-primitives` | Shared tokenomics + reward/split/merkle math (single source of truth, on- and off-chain).                                                          |
| `scripts/run-node.sh`     | Turn a **home device behind NAT** into a node: Xray + Tor + `frpc` (reverse tunnel to a relay) + control plane. Earns `$WEFT` for traffic served. |
| `scripts/deploy-node.sh`  | One-command **VPS** node setup: Xray (VLESS+Reality) + Tor + the control plane; both connection modes; prints the links.                           |
| `services/control-plane/` | Per-node token-gating service: mints personal links, meters traffic (Xray stats), enforces the `$WEFT` budget, verifies `pay_traffic` settlement. |
| `services/`               | Other TypeScript services + CLIs: registry provisioning, the settlement aggregator, the node-directory indexer, governance tooling, and genesis.   |
| `sdk/`                    | `@weft/sdk` — typed Solana clients for every program, plus the shared math.                                                                       |
| `clients/desktop/`        | The cross-platform desktop VPN app.                                                                                                                |

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
