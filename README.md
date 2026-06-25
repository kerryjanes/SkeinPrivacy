# Weft

**A decentralized VPN on Solana.** Anyone runs a node, carries other people's traffic, and earns
`$WEFT`. Users pay `$WEFT` for access (0.1 `$WEFT`/GB). You connect with apps you already
trust — **V2Box, Happ, sing-box, Hiddify, Streisand** — because the transport is standard
**VLESS + Reality** (masquerades as ordinary HTTPS, so DPI, incl. Russia's TSPU, can't see it).

Two connection modes per node: **1-hop** (direct, fast) and **multihop** (routed through **Tor**,
maximum privacy, slower).

---

## Use it

You need a Solana wallet with `$WEFT` — that balance is your traffic budget.

1. Open the **cabinet** → [weftnetwork.net/app](https://www.weftnetwork.net/app), connect your
   wallet, go to **access**.
2. Copy your personal link — **1-hop** or **multihop** (each is metered to your wallet).
3. Paste it into any VLESS client (Happ / V2Box / …) and press **Connect**.

When your `$WEFT` runs out the link stops; top up (or earn it by running a node) and it resumes.
On devnet, click **Get test $WEFT** in the cabinet to try it.

---

## Run a node & earn

A node carries traffic under **your wallet** and earns `$WEFT` **for the bytes it actually
serves** (weighted by reputation 0.5×–2.0×, geo demand up to +50%, stake +20% at 10k `$WEFT`).

1. **Start the node software** on your machine:
   - **Home device** (PC / router, behind NAT): `./scripts/run-node.sh`
   - **Public VPS** (has its own IP): `sudo ./scripts/deploy-node.sh` (add `ya.ru` for DPI-heavy markets)

   Either way it installs Xray (VLESS+Reality) + Tor + the control plane and prints your public
   endpoint.

2. **Register it** in the cabinet → **deploy**: paste that endpoint → it mints your node on-chain
   (a cNFT) under your wallet.
3. **Claim earnings** in the cabinet → **rewards**, once the network has settled an epoch.

---

## How it works

- **Transport:** Xray-core (VLESS + Reality) for DPI resistance + the Tor daemon for the multihop
  onion path. No custom protocol.
- **Home nodes behind NAT** reach users via a public **relay** (reverse tunnel, the
  Tailscale-DERP / Tor-relay model) — the relay forwards traffic to your node; you exit at home.
- **Token-gating:** each node runs a **control plane** that mints a personal link per wallet,
  meters its traffic, and cuts a user off when their `$WEFT` is spent.
- **On-chain:** every node is a compressed NFT in the registry; traffic is paid via `pay_traffic`
  (70% nodes / 20% burn / 10% treasury) and settled to nodes each epoch. `$WEFT` also stakes
  (priority + bonus) and votes (DAO).

---

## Repository layout

| Path                      | What                                                                                                                    |
| ------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `programs/`               | 7 Anchor programs: vesting, node registry (cNFT), staking, reputation, rewards settlement, governance, IDO distributor. |
| `crates/weft-primitives` | Shared tokenomics + reward/split/merkle math (single source of truth, on- and off-chain).                               |
| `scripts/run-node.sh`     | Turn a **home device behind NAT** into a node (Xray + Tor + `frpc` reverse tunnel + control plane).                     |
| `scripts/deploy-node.sh`  | One-command **VPS** node (Xray + Tor + control plane, both modes).                                                      |
| `services/control-plane/` | Per-node token-gating: mints personal links, meters traffic, enforces the `$WEFT` budget.                              |
| `services/`               | The rest: registry provisioning, the settlement aggregator, the node-directory indexer, genesis.                        |
| `sdk/`                    | `@weft/sdk` — typed Solana clients for every program + shared math.                                                    |
| `clients/desktop/`        | Cross-platform desktop app.                                                                                             |

On-chain program IDs (devnet): `node-registry 6dsqVjMmczosqNk2kaFHa33ut9ZUAwazgUagPKk5tUgd` ·
`staking 86FwTDBau7T289G9Fnkjn34g7NN3furoGEDwFsLVXzTK` ·
`reputation 6Nwa73bqP56LNQwWEKJWAp4A5RJKSMBzFxdxtuq3Y86u` ·
`rewards-settlement BMQZKvCbq8qcZFWWGt1S7ZXg8odZcMbUA3oaNKnQi7mz` ·
`governance q3K9krqiQDL7WHVUzLZrjJLgsM53vSrcfNRTzsVE6eA` ·
`token-distributor 2vXouVhktgRQhBhuvUMpi6hCryEtAsJeHJMKG9QgxpzV` ·
`vesting FCFZNb2Kqh7ScjikKp73W7BcsfusrZ1hTBhc61Macdsv`

## Build from source

Requires Rust (stable), Node 22+ with pnpm, and the Anchor/Solana toolchain.

```sh
pnpm install && pnpm build      # JS workspaces (sdk, services, cabinet)
cargo test --workspace          # Rust + program tests
anchor build                    # the on-chain programs
```

See [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md). [MIT](LICENSE).
