# PROJECT.md — Weft Privacy (living project memory)

Single-source status for a fresh session. Keep it current. **No secrets here** — only public
keys and file locations.

## What this is

A Solana DePIN VPN. Home nodes carry traffic; users pay in the launch token ($WEFT); a single
Anchor program (`weft`) holds custody, settlement, staking, and node identity (Bubblegum V2
compressed NFTs). Off-chain services meter usage and settle in compact on-chain checkpoints.

- On-chain: one program, instructions for `initialize_core`, escrow deposit/pay/withdraw,
  `stake`/`unstake`, `post_epoch`, `claim`, `dispute`, `register_node` (cNFT). Vaults are PDAs.
- Off-chain (ESM, @solana/kit): **control-plane** (per-wallet VPN link, quota from escrow),
  **aggregator** (poster + node-reward payout), **indexer** (node directory), **buyback-burn**
  (SOL→$WEFT→burn worker, off by default). **registry-provision** = init + cNFT provisioning + node agent.
- Frontend: `weft-web/cabinet` (Vite, Vercel, base `/app/`). Reads mint/vault/treasury/decimals
  from the on-chain Distributor **at runtime** — no rebuild when the token launches.
- Economics: every $WEFT payment splits **70% nodes / 20% burned on-chain (BurnChecked) / 10% treasury**.

## Current state (2026-07-03)

- **Mainnet program was CLOSED.** The original id `6riawCPVNE6sjMC6dgqkB2FxjXXFMXzuuy1pQRimk8Yd`
  was `solana program close`d; **4.40671704 SOL was recovered to the admin** wallet
  `9AY1on6okCYep7uKVTmJQbGHa7uqv5sQ4AwfuJhEkqJw` (now ~6.53 SOL). Nothing was initialized on
  mainnet, so no funds were ever stuck. The id is **dead** — a relaunch needs a fresh id.
- **The program is still deployed on devnet** at `6riawCPV…` (initialized with a classic-SPL
  test mint) — used for the node directory + cNFT demo.
- **Relaunch is deferred to a rebrand** (new name/symbol/look, new token, fresh program id).
- Backend on the relay VPS (`root@13.140.2.111`) still runs on **devnet**. Services:
  `weft-control-plane`, `weft-aggregator`, `xray`. Operational/poster key on the box:
  `DEg6vvwNmkhaV9aTUaEUbhCG5AbFKNvGq8egiqScF1nq` (node-reward payout wallet + epoch poster).

## The Token-2022 incident — post-mortem (do not repeat)

**What happened:** at the coordinated launch, the token minted on pump.fun was **Token-2022**
(owner `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`), not classic SPL. The on-chain program was
already token-agnostic (uniform `token_interface`), but **every off-chain client hardcoded the
classic Token program**, so the launch could not proceed on the real token and the window was lost.

**Root cause:** assuming the token program instead of detecting it. pump.fun now mints Token-2022 by default.

**Fix (shipped):** the whole stack now reads the reward mint's **owner program from chain at
runtime** and threads it into every ATA derivation, transfer, burn, and instruction:
- `registry-provision/src/initCore.ts` — guard accepts classic **or** Token-2022, **rejects**
  economy-breaking extensions (transfer fee, transfer hook, permanent delegate, default-frozen,
  interest, non-transferable, confidential), and passes the detected `tokenProgram` to init.
- `aggregator` (`cli.ts` resolves the owner → `payout.ts`, `pay.ts` PayConfig, `server.ts`).
- `cabinet/src/lib/chain.ts` — memoized `resolveTokenProgram(mint)`; `ata()`, `rewardConfig()`,
  deposit/settle/withdraw and `Staking.tsx` stake all token-program-aware.
- `control-plane/src/faucet.ts` (devnet test mint) and `buyback-burn` (web3.js v1) too.

**Proven:** full money flow rehearsed on a local validator with a real Token-2022 mint —
`core:init → deposit → pay` produced the exact **70% vault / 10% treasury / 20% on-chain burn
(mint supply decrease)**, and `TokenPayout` credited a node (ATA auto-created). CI regression test
in `aggregator/test/pay.test.ts` asserts the settlement builders never fall back to classic.

## Relaunch checklist (at the rebrand — mainnet spend happens here)

The program is fully T2022-safe; a relaunch is a **fresh deploy + id rotation**, not a code change.
1. New program keypair; `declare_id!` (`programs/weft/src/lib.rs:19`) + `Anchor.toml` (3 clusters).
   Build with `anchor build --ignore-keys` (never commit the new secret).
2. Regenerate **both** SDKs from the new IDL: `pnpm codegen` (Weft/sdk) **and** the separate
   `weft-web/sdk` (`codegen.mjs`). Then `pnpm -r build` (rebuilds services + the id-baked bundles).
3. **Rebuild + recommit the operator bundles** — nodes `curl` them from GitHub raw:
   `services/{aggregator,control-plane,registry-provision}/dist/*.mjs`. Grep the tree for the OLD
   id before deploying (past landmine: a stale `HFt8Bm7r…` id lived in a committed bundle).
4. Deploy the program to mainnet; then `./scripts/mainnet-launch.sh <CA> --yes`
   (`core:init` detects the CA's token program) → `./scripts/mainnet-cutover.sh <CA> --yes`
   (derives the distributor PDA from the SDK — rotation-safe). Fund `DEg6vvw…` with $WEFT for node payouts.
5. Rebuild the cabinet from mainnet env; redeploy to Vercel; verify `/price` + `/app`.

**Rebrand swap-list** (keep "Weft" until the new brand lands): collection name + URI in
`registry-provision/src/provision.ts` (`'Weft Nodes'`, `COLLECTION_URI`), node metadata +
symbol in `registry-provision/src/nft.ts` (`NODE_METADATA_URI`), domain via `WEFT_HOST` /
`VITE_WEFT_RELAY_HOST` env (default `vpn.weftnetwork.net`), and the hosted `nft/*.json`.

## Verify the Token-2022 money flow (pre-relaunch regression)

`cargo test --workspace` + `pnpm -r build` + `pnpm --filter @weft/aggregator test` must be green.
For an end-to-end check, run a local validator with the program at its id, create a Token-2022
mint (`spl-token create-token --program-2022 --decimals 6`), run `core:init` (env
`WEFT_CLUSTER=localnet`), then a deposit + `pay_traffic_from_escrow` and assert the 70/20/10 split
and the 20% supply burn. (Optional future hardening: a LiteSVM Rust test in CI for the same path.)

## Keys, wallets, secrets (locations only — never commit the values)

- **Admin / upgrade authority** `9AY1on6…` — signs deploys + admin from the local machine only,
  key at `~/.config/solana/weft-admin.json`. Never on a server.
- **Operational/poster** `DEg6vvw…` — on the VPS at `/etc/weft/authority.json`; node payouts + epoch posts.
- Helius RPC key: local `~/Documents/helius/config.json` (read at runtime; ok to expose on the frontend only).
- Reality/Xray keys, GitHub token: never in code; injected server-side / masked in output.

## Key commands

- Typecheck all: `pnpm -r build`. Tests: `cargo test --workspace`, `pnpm -r test`.
- Bundles (operators curl these): `pnpm --filter @weft/<svc> bundle`.
- Launch (fresh-deployed program): `./scripts/mainnet-launch.sh <CA> --yes` then `./scripts/mainnet-cutover.sh <CA> --yes`.
- Recover program rent (pre-production only): `solana program close <id> --recipient 9AY1on6… --bypass-warning`.
