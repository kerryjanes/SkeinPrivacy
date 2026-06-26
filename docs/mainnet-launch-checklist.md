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
- Run the aggregator with `WEFT_CLUSTER=mainnet-beta`, `WEFT_RPC`, and production receipt ingestion.
- Confirm `/receipts` ingestion builds proofs, `post_epoch` lands on-chain, `/proof` returns claimable proofs, and `claim` succeeds after the dispute window.
- Submit the same `pay_traffic` signature twice against a node control-plane and verify the second attempt is rejected.

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
- Verify wallet connect, node registration, access provisioning, payment settlement, staking, rewards, and governance flows with a small mainnet canary amount.

## Secrets

- Confirm `.env*`, keypairs, Vercel local env, manifests, node keys, founder links, and poster/dispute keys are not committed.
- Rotate any key or token that has ever appeared in logs, screenshots, chat, shell history, or public deployment output.
