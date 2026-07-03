# Weft — mainnet launch runbook

> **Status (2026-07-03):** the first launch was **aborted** — pump.fun minted a **Token-2022**
> token and the clients assumed classic SPL. The whole stack is now token-program-agnostic
> (see the post-mortem in `PROJECT.md`). The mainnet program was **closed** and its 4.4 SOL
> recovered to the admin. **Relaunch happens under a rebrand with a fresh program id.** This
> runbook is the launch-window procedure once that fresh program is deployed.

## Before the window — fresh deploy (see `PROJECT.md` → "Relaunch checklist")

The old id `6riawCPV…` is dead. A relaunch first: new program keypair → `declare_id!` + `Anchor.toml`
→ `anchor build --ignore-keys` → regenerate **both** SDKs → `pnpm -r build` → **rebuild + recommit
the operator bundles** (`services/*/dist/*.mjs`, curled from GitHub raw) → deploy the program to
mainnet → rebuild the cabinet from mainnet env. Grep the tree for the old id before deploying.

## State to prefill

| Piece                       | Value                                          | Notes                                                                                    |
| --------------------------- | ---------------------------------------------- | ---------------------------------------------------------------------------------------- |
| Program                     | fresh id (rotated at rebrand)                  | Deploy to mainnet under the new `declare_id!` **before** the two launch commands.        |
| Admin / deploy authority    | `9AY1on6okCYep7uKVTmJQbGHa7uqv5sQ4AwfuJhEkqJw` | Key at `~/.config/solana/weft-admin.json` (~6.53 SOL). Never copied to the VPS.         |
| Reward token (CA)           | pump.fun mint — **classic SPL or Token-2022**  | `core:init` detects the mint's owner program at runtime; rejects fee/hook extensions.    |
| Frontend                    | https://weftnetwork.net (+ /app)              | Reads mint/vault/treasury/decimals from the Distributor at runtime → no rebuild at launch. |
| Relay VPS                   | `root@13.140.2.111`                            | control-plane, aggregator, xray (still devnet until cutover).                             |
| Poster + node-payout wallet | `DEg6vvwNmkhaV9aTUaEUbhCG5AbFKNvGq8egiqScF1nq` | `/etc/weft/authority.json` on the VPS. Posts epochs + pays nodes.                        |
| RPC                         | Helius mainnet                                 | Key read at runtime from `~/Documents/helius/config.json` (never committed).             |

## Launch (on `go`, with the CA — program already deployed)

```bash
cd ~/Documents/Crypto/Weft
CA=<pump.fun mint>

# 1. On-chain: treasury ATA -> initialize_core(reward_mint=CA) -> provision cNFT collection+tree
#    initCore detects whether CA is classic SPL or Token-2022 and threads it through.
./scripts/mainnet-launch.sh "$CA" --yes

# 2. Relay VPS: control-plane + aggregator -> mainnet + CA, wipe devnet state, restart
#    (derives the distributor PDA from the SDK — correct for the rotated id)
./scripts/mainnet-cutover.sh "$CA" --yes
```

Both are idempotent. Launch spend ~0.23 SOL (cNFT merkle-tree rent ~0.22 is non-recoverable).
`mainnet-launch.sh` must run before `mainnet-cutover.sh` (the aggregator won't boot without the
initialized distributor; the cutover checks for it and refuses otherwise).

## Post-launch funding (before nodes withdraw)

Node rewards are paid directly from the payout wallet. Forward $WEFT to it:

```bash
spl-token transfer "$CA" <amount> DEg6vvwNmkhaV9aTUaEUbhCG5AbFKNvGq8egiqScF1nq \
  --fund-recipient --url "$WEFT_RPC_URL" --owner ~/.config/solana/weft-admin.json
```

Payouts are capped at the wallet's balance — an underfunded wallet only _pauses_ withdrawals,
never overpays or goes negative.

## Verify (after both commands)

- `curl https://vpn.weftnetwork.net:8089/price` → `mint` = CA, `faucet:false`.
- https://weftnetwork.net/app → connect a mainnet wallet; distributor resolves the token;
  deposit → copy VPN link → connect → traffic meters and debits balance.
- Register a node from the cabinet → node cNFT lands in the wallet.
- Check treasury/reward-vault balances and a node's ATA before/after a withdrawal, and confirm a
  payment burns 20% (mint supply decreases).

## Recovery (abort a mainnet deploy — returns program rent to admin, kills the program id)

```bash
solana program close <program-id> \
  --recipient 9AY1on6okCYep7uKVTmJQbGHa7uqv5sQ4AwfuJhEkqJw \
  -k ~/.config/solana/weft-admin.json --url mainnet-beta --bypass-warning
```

> Already executed once for `6riawCPV…` on 2026-07-03 (4.40671704 SOL reclaimed). Pre-production
> only — never close a live program in production (it changes the id and breaks every consumer).
