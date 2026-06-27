# Mainnet Launch Checklist

This checklist is the manual launch gate. Do not launch while any item is unknown.

## On-chain deployment

- Deploy all Anchor programs to mainnet-beta from a clean toolchain.
- Update every `declare_id!` to the deployed mainnet program id.
- Add `[programs.mainnet]` to `Anchor.toml`.
- Regenerate `@weft/sdk` from the final IDLs and confirm the generated program addresses match mainnet.
- Verify deployed program buffers/upgrade authorities are controlled by the intended multisig or are intentionally immutable.

## Genesis

- Set `WEFT_CLUSTER=mainnet-beta`.
- Set `WEFT_RPC_URL` and `WEFT_TGE_TS` explicitly.
- Set liquid custody owners:
  - `WEFT_TREASURY_OWNER`
  - `WEFT_EMISSIONS_OWNER`
  - `WEFT_IDO_OWNER`
- Set vesting schedule owners:
  - `WEFT_TEAM_BENEFICIARY` and `WEFT_TEAM_AUTHORITY`
  - `WEFT_ECOSYSTEM_BENEFICIARY` and `WEFT_ECOSYSTEM_AUTHORITY`
  - `WEFT_MARKETING_BENEFICIARY` and `WEFT_MARKETING_AUTHORITY`
- After genesis, verify token supply, decimals, retired mint authority, null freeze authority, custody balances, vesting vault balances, and manifest addresses on-chain.

## Registry

- Set `WEFT_CLUSTER=mainnet-beta` and `WEFT_RPC_URL`.
- Provision `services/registry-provision/manifests/mainnet-beta.json`.
- Verify collection URI, collection update authority, active tree, tree shard, registry PDA, and tree depth.
- Rotate registry metric authorities to the intended staking/reputation authorities.

## Settlement And Rewards

- Initialize distributor with the mainnet mint, reward vault, treasury, poster authority, and dispute authority.
- Verify dispute/clawback windows and protocol split parameters.
- Choose and document exactly one mainnet metering/custody model before launch:
  - **On-chain settlement model:** users periodically sign `pay_traffic_from_escrow`; node rewards are paid from the on-chain reward vault/proof flow.
  - **Off-chain ledger model:** user prepaid balances, usage, refunds, and node payouts are all reconciled by backend custody with signed withdrawals/refunds.
- Do not mix off-chain usage with unrestricted on-chain `withdraw_escrow` for production without an explicit risk cap: a user can withdraw escrow before off-chain usage is settled.
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
  - `VITE_WEFT_REGISTRY`
  - `VITE_WEFT_COLLECTION`
  - `VITE_WEFT_MERKLE_TREE`
  - `VITE_WEFT_TREE_SHARD`
- Verify wallet connect, node registration, escrow deposit, access provisioning, escrow settlement, escrow withdraw, staking, rewards, and governance flows with a small mainnet canary amount.
- Confirm no mainnet build shows devnet-only actions: keypair paste connect, test `$WEFT` faucet, SOL faucet, trusted totals, or test labels.

## Mainnet Economy Gate

- User price and node payout must reconcile before launch. With the current configured rate of `1000 $WEFT / GB`, the enforced maximum node payout is `700 $WEFT / GB`; the remaining `300 $WEFT / GB` is the burn/treasury/infrastructure margin.
- Never pay node rewards from fresh minting on mainnet.
- If rewards are paid by backend custody, reconcile daily:
  - total prepaid user liabilities,
  - total unpaid node earnings,
  - payout wallet token balance,
  - treasury/burn transfers,
  - aggregate bytes served by nodes vs bytes charged to users.
- Set minimum withdrawal and payout batching thresholds so Solana fees cannot exceed the reward being paid.
- Cap canary traffic and per-wallet unpaid exposure until refund/settlement reconciliation has been audited.

## Secrets

- Confirm `.env*`, keypairs, Vercel local env, manifests, node keys, founder links, and poster/dispute keys are not committed.
- Rotate any key or token that has ever appeared in logs, screenshots, chat, shell history, or public deployment output.
