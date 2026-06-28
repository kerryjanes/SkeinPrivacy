# Mainnet Launch Checklist

This checklist is the manual launch gate. Do not launch while any item is unknown.

## On-chain deployment

- Run `./scripts/mainnet-preflight.sh` before funding or sending any mainnet transaction.
- Deploy the single Anchor program `programs/weft` to mainnet-beta from a clean toolchain.
- Do not deploy legacy programs from `legacy-programs/*`; they are not part of the mainnet MVP.
- Update `declare_id!` in `programs/weft/src/lib.rs` to the deployed mainnet program id.
- Add `[programs.mainnet]` to `Anchor.toml`.
- Regenerate `@weft/sdk` from the final `weft` IDL and confirm `WEFT_PROGRAM_ADDRESS` matches mainnet.
- Verify the deployed program buffer and upgrade authority are controlled by the intended multisig or are intentionally immutable.
- Current preflight size estimate for the single `weft` program is about `529,912` bytes / `3.6890784 SOL` rent-exempt programdata. Keep the deploy wallet funded above this for buffer accounts, fees, and initialization accounts; this is not a seven-program deploy.

## Genesis

- Set `WEFT_CLUSTER=mainnet-beta`.
- Set `WEFT_RPC_URL` explicitly.
- Set `WEFT_MINT` to the mainnet `$WEFT` mint.
- Set `WEFT_TREASURY_TOKEN_ACCOUNT` to the treasury token account for this mint.
- Set `WEFT_POSTER_AUTHORITY` to the aggregator/poster wallet.
- Set `WEFT_DISPUTE_AUTHORITY` to the dispute/slash authority.
- Run `pnpm --filter @weft/registry-provision core:init`.
- Verify registry, staking config, distributor, and reward vault PDA addresses on-chain.
- Verify token supply, decimals, retired mint authority, null freeze authority, and treasury balances on-chain.

## Settlement And Rewards

- Use the single-program escrow model for mainnet MVP:
  - users deposit `$WEFT` into `PaymentEscrow`;
  - `pay_traffic_from_escrow` debits usage;
  - 70% goes to the node reward vault, 20% to treasury, 10% is burned;
  - the aggregator posts epoch Merkle roots from off-chain bandwidth receipts;
  - node operators claim from the reward vault after the dispute window.
- Verify dispute/clawback windows and the 70/20/10 split parameters.
- Verify Payment Escrow:
  - `deposit_escrow` creates exactly one escrow PDA and escrow vault per wallet.
  - `pay_traffic_from_escrow` debits prepaid balance and applies the governed node/treasury/burn split.
  - `withdraw_escrow` returns only unused prepaid balance to the wallet.
- Verify burn accounting: total token supply decreases by exactly the burn share for direct payments and escrow-settled payments.
- Run the aggregator with `WEFT_CLUSTER=mainnet-beta`, `WEFT_RPC`, and production receipt ingestion.
- Confirm `WEFT_TRUSTED_TOTALS`, `WEFT_TRUSTED_OPERATOR`, `WEFT_TRUSTED_NODE_ID`, and `WEFT_TRUSTED_BYTES` are unset on mainnet.
- Set `WEFT_PAYOUT_KEYPAIR` and `WEFT_PAYOUT_STORE` explicitly if using the current earned-ledger payout backend.
- Set `WEFT_PAYOUT_RESERVE` to the minimum token balance the payout wallet must retain after any withdrawal.
- Confirm `/withdraw-earned` rejects unsigned requests and invalid signatures, and accepts only a fresh `/withdraw-earned/challenge` signed by the node operator wallet.
- Confirm the payout wallet balance is greater than total unpaid earned balances plus `WEFT_PAYOUT_RESERVE`.
- Confirm reward builds cap every node payout to the collected node-share budget (`700 WEFT / GB` at the current `1000 WEFT / GB` user price). Bonus multipliers may rank/boost up to this cap, but must not create liabilities above user-paid node-share unless a separate emissions budget is enabled and reconciled.
- Confirm `/receipts` ingestion builds proofs, `post_epoch` lands on-chain, `/proof` returns claimable proofs, and `claim` succeeds after the dispute window.
- Submit the same settlement signature twice against a node control-plane and verify the second attempt is rejected.
- Keep direct `pay_traffic` disabled in the public UX unless intentionally preserving it as a legacy/manual settlement path.

## Nodes And Web

- For VPS/home nodes, set `WEFT_CLUSTER=mainnet-beta`, `WEFT_RPC`, and `WEFT_MINT` explicitly.
- Confirm `WEFT_FAUCET_KEYPAIR` is unset on every mainnet control-plane.
- Confirm home node traffic exits through the user node first, with VPS fallback only when no live user node is available.
- Configure cabinet env:
  - `VITE_WEFT_CLUSTER=mainnet-beta`
  - `VITE_WEFT_RPC_URL`
  - `VITE_WEFT_AGGREGATOR_URL`
  - `VITE_WEFT_CONTROL_PLANE_URL`
  - `VITE_WEFT_RELAY_HOST`
- Verify wallet connect, node registration, escrow deposit, access provisioning, escrow settlement, escrow withdraw, staking, and rewards with a small mainnet canary amount.
- Confirm no mainnet build shows devnet-only actions: keypair paste connect, test `$WEFT` faucet, SOL faucet, trusted totals, or test labels.

## Mainnet Economy Gate

- User price and node payout must reconcile before launch. With the current configured rate of `1000 $WEFT / GB`, the enforced maximum node payout is `700 $WEFT / GB`; the remaining `300 $WEFT / GB` is the burn/treasury/infrastructure margin.
- Never pay node rewards from fresh minting on mainnet.
- Reconcile daily:
  - total prepaid user liabilities,
  - total unpaid node earnings,
  - reward vault token balance,
  - treasury/burn transfers,
  - aggregate bytes served by nodes vs bytes charged to users.
- Set minimum withdrawal and payout batching thresholds so Solana fees cannot exceed the reward being paid.
- Cap canary traffic and per-wallet unpaid exposure until refund/settlement reconciliation has been audited.

## Secrets

- Confirm `.env*`, keypairs, Vercel local env, manifests, node keys, founder links, and poster/dispute keys are not committed.
- Rotate any key or token that has ever appeared in logs, screenshots, chat, shell history, or public deployment output.
