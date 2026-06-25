# Weft

**A decentralized VPN on Solana.** Weft is a DePIN network: anyone runs a node from a home device
or VPS, carries other people's traffic, and earns `$WEFT`; users pay `$WEFT` for access. The
transport is standard **VLESS + Reality**, so you connect with apps you already trust and DPI can't
tell it from normal HTTPS.

Each node offers **two connection modes**:

- **1-hop** — direct VLESS + Reality exit. Fast; traffic egresses at the node.
- **multihop** — VLESS + Reality, then routed through the **Tor network**. Onion routing, maximum
  privacy (the exit never learns who you are); slower, egresses at a Tor exit.

Key properties:

- **Data plane:** battle-tested **Xray-core (VLESS + Reality)** for DPI-resistant transport + the
  **Tor** daemon for the multihop onion path. No custom protocol — Weft's value-add is the
  polished node, the relay layer for home devices, and the on-chain incentive/registry.
- **Reality masquerade:** the connection looks like ordinary HTTPS to a real site, so DPI (incl.
  Russia's TSPU) can't fingerprint it. Compatible clients: **V2Box, Happ, sing-box, Hiddify,
  Streisand** (iOS · Android · macOS · Windows · Linux).
- **Token-gated access:** every node runs a **control plane** that mints a _personal_ link per
  wallet, meters its traffic via Xray's stats API, and **cuts you off once you've used what your
  `$WEFT` pays for** (0.1 `$WEFT`/GB) — restoring you when you top up or earn `$WEFT`. Your
  balance is your budget; no flat "one shared unlimited link".
- **`$WEFT` (SPL token):** pay for traffic, reward node operators, stake on nodes (priority +
  bonus), and vote in the DAO.
- **Registry:** every node is a compressed NFT on Solana carrying its geo, capabilities, stake and
  reputation.

---

## Use Weft as a VPN

You don't need to run a node to use the network — just a Solana wallet holding `$WEFT`, which is
your traffic budget (0.1 `$WEFT`/GB).

### 1. The cabinet + an app you already use (recommended)

1. Open the **cabinet** → [weftnetwork.net/app](https://www.weftnetwork.net/app) and connect your
   wallet, then go to **access**.
2. It shows your `$WEFT` budget (balance · used · remaining) and your two **personal** links
   (`1-hop` = fast, `multihop` = Tor / max privacy), each metered to your wallet. Copy one.
3. Import it into any VLESS client (**V2Box / Happ / sing-box / Hiddify / Streisand**) and press
   **Connect**.

When your `$WEFT` runs out the link stops; top up your wallet (or earn `$WEFT` by running a node)
and it resumes. On **devnet**, click **Get test $WEFT** in the cabinet to try the whole flow.

_Advanced / scripted:_ the cabinet just calls a node's control plane —
`curl -X POST https://<node>:8089/provision -d '{"wallet":"<YOUR_SOLANA_PUBKEY>"}'`.

### 2. The Weft desktop app

Download from [**Releases**](https://github.com/kerryjanes/WeftNetwork/releases), load your wallet,
and press **Connect** — it provisions your personal link and shows your `$WEFT` budget live.
Nothing to paste.

---

## Run a node & earn

A home device becomes a **1-hop node** that carries traffic under **your wallet** and earns `$WEFT`
**for the bytes it actually serves** (metered at the node). Rewards are weighted by **reputation**
(0.5×–2.0×), **geo demand** (up to +50%), and **stake** (+20% at 10,000 `$WEFT`). Multihop is always
served over **Tor** by infrastructure nodes — your device never becomes a Tor/relay node.

**Step 1 — register + pay on the site.** In the cabinet → **deploy**, connect your wallet and press
**Register** (your wallet covers the one-time on-chain rent; the region is auto-detected). You get
back a single **node key** — copy it. This is the only step that touches the chain.

**Step 2 — start the node software.** On the device you want to turn into a node (PC / router /
always-on box, behind NAT), pass the key **once**:

```sh
./scripts/run-node.sh <your-node-key>      # first run only
./scripts/run-node.sh                      # afterwards — the key is saved locally
```

A home device can't accept inbound connections, so the script does what Tailscale-DERP /
Cloudflare-Tunnel / Tor relays do: it dials **out** to a public **rendezvous relay** (`frpc`), which
exposes your node at the public `relay:port` your registration committed. Users connect there; the
relay forwards to your home Xray; your traffic exits at home (1-hop). It installs Xray + `frpc` + the
control plane as **persistent services** (systemd on Linux, launchd on macOS), so the node survives
reboots, crashes, and closing the terminal (auto-restart).

Stop being a node any time with `./scripts/stop-node.sh` (no key needed — it leaves the live list but
stays registered, so restarting with `./scripts/run-node.sh` never re-pays; add `--purge` to remove
it entirely). A **public VPS** can serve both modes directly with `sudo ./scripts/deploy-node.sh`.

**Step 3 — claim earnings.** In the cabinet → **rewards**, once the network has settled an epoch:
pick your node + an epoch and claim your `$WEFT`.

---

## How it works

```
1-hop:    [user] --VLESS+Reality--> [relay:port] --frp tunnel--> [home node Xray] --> internet
multihop: [user] --VLESS+Reality--> [infra node Xray] --> Tor network --> exit
```

- **Transport:** Xray-core (VLESS + Reality) for DPI resistance; the Tor daemon provides the
  multihop onion path. No custom data-plane code.
- **1-hop vs multihop:** home devices register as **1-hop** nodes only (fast, direct exit) — they
  never run Tor. **Multihop** is always served over the **Tor network** by infrastructure nodes, so a
  home operator never becomes a Tor/relay node.
- **Relay layer:** home nodes behind NAT are reached via public **relays** (reverse tunnel, the
  Tailscale-DERP / Tor-relay model) that expose them at a public `relay:port`.
- **Token-gating:** each node's control plane mints a personal link per wallet, meters its traffic,
  and cuts a user off when their `$WEFT` is spent — restoring them on top-up.
- **Settlement:** traffic is paid via `pay_traffic` (split **70% nodes / 20% burn / 10% treasury**);
  every epoch the aggregator tallies each node's served bytes, posts a Merkle root on-chain, and
  nodes claim their share.

---

## Repository layout

| Path                      | What                                                                                                                      |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `programs/`               | 7 Anchor programs: vesting, node registry (cNFT), staking, reputation, rewards settlement, governance, IDO distributor.   |
| `crates/weft-primitives` | Shared tokenomics + reward/split/merkle math (single source of truth, on- and off-chain).                                 |
| `scripts/run-node.sh`     | Turn a **home device behind NAT** into a 1-hop node (persistent service): Xray + `frpc` (reverse tunnel) + control plane. Takes the node key from cabinet → deploy. |
| `scripts/stop-node.sh`    | Stop a home node started by `run-node.sh` (`--purge` to remove it entirely).                                              |
| `scripts/deploy-node.sh`  | One-command **VPS** node: Xray (VLESS+Reality) + Tor + control plane, both modes.                                         |
| `services/control-plane/` | Per-node token-gating: mints personal links, meters traffic (Xray stats), enforces the `$WEFT` budget.                   |
| `services/`               | The rest: registry provisioning, the settlement aggregator, the node-directory indexer, governance tooling, genesis.      |
| `sdk/`                    | `@weft/sdk` — typed Solana clients for every program + the shared math.                                                  |
| `clients/desktop/`        | The cross-platform desktop VPN app.                                                                                       |

### On-chain program IDs (Solana devnet)

```
node-registry        6dsqVjMmczosqNk2kaFHa33ut9ZUAwazgUagPKk5tUgd
staking              86FwTDBau7T289G9Fnkjn34g7NN3furoGEDwFsLVXzTK
reputation           6Nwa73bqP56LNQwWEKJWAp4A5RJKSMBzFxdxtuq3Y86u
rewards-settlement   BMQZKvCbq8qcZFWWGt1S7ZXg8odZcMbUA3oaNKnQi7mz
governance           q3K9krqiQDL7WHVUzLZrjJLgsM53vSrcfNRTzsVE6eA
token-distributor    2vXouVhktgRQhBhuvUMpi6hCryEtAsJeHJMKG9QgxpzV
vesting              FCFZNb2Kqh7ScjikKp73W7BcsfusrZ1hTBhc61Macdsv
```

`$WEFT` mint (devnet): `8AYQEuGHXXwndyfLCY4quyNoMxTPxzh2CJv6DwpDaC8i` · the live launch node +
relay run at `vpn.weftnetwork.net`.

---

## Build from source

Requires Rust (stable), Node 22+ with pnpm, and the Anchor/Solana toolchain for the programs.

```sh
pnpm install && pnpm build      # JS workspaces (sdk, services, cabinet)
cargo test --workspace          # Rust unit + program (LiteSVM) tests
anchor build                    # the on-chain programs
```

## Contributing & security

Issues and pull requests welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). Please report security
issues privately as described in [SECURITY.md](SECURITY.md).

## License

[MIT](LICENSE).
