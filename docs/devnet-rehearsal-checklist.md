# Devnet Rehearsal Checklist

Run this before any mainnet canary. The goal is to exercise the same launch flow on devnet with no manual code patches.

## 1. Static Readiness

- `pnpm -r build` passes.
- `pnpm -r test` passes.
- `anchor build` passes.
- `cargo test --workspace --lib --tests` passes.
- `pnpm -C ../weft-web/cabinet build` passes.
- `pnpm -F @weft/control-plane bundle` rebuilds `services/control-plane/dist/control-plane.mjs`.

## 2. Devnet Configuration

- `Anchor.toml` devnet program IDs match each program `declare_id!`.
- `services/genesis/manifests/devnet.json` exists and points to mint `Hfvwx9F5NDzMCyywJZJsFVX83XaXnLNntCdk21h7Bmcy`.
- `services/registry-provision/manifests/devnet.json` exists and points to registry `6tw8x8sm18fz5jMsHfVxvPbCQm4Nf8e6gqKUn84pBjyW`, collection `DLeBsmxSNB1RmcPrGSWm5J5tPqXjAKWVfqkVpCpCZdqY`, tree `4RJP3AJ6NNoqjTCxjeJi2Erw3wwJJoHN3jpwUrSetJw5`, and shard `8dvswMfvXUBg2YZSNoNijYaQBRNKqzzhyDToZq1Day8E`.
- Control-plane devnet env uses:
  - `WEFT_CLUSTER=devnet`
  - `WEFT_RPC` or `WEFT_RPC_URL`
  - devnet `$WEFT` mint
  - optional transfer-faucet keypair funded with devnet `$WEFT`; do not use `MintTo` because mint authority is retired.
- Cabinet devnet env uses:
  - `VITE_WEFT_CLUSTER=devnet`
  - `VITE_WEFT_RPC_URL`
  - `VITE_WEFT_CONTROL_PLANE_URL`
  - `VITE_WEFT_AGGREGATOR_URL`

## 3. Devnet On-chain Checks

- Every configured devnet program account exists and is executable. Verified for the fresh devnet IDs now merged into `main`.
- Devnet `$WEFT` mint exists, has 9 decimals, full supply `1000000000000000000`, retired mint authority, and null freeze authority. Verified for `Hfvwx9F5NDzMCyywJZJsFVX83XaXnLNntCdk21h7Bmcy`.
- Registry PDA exists. Verified for `6tw8x8sm18fz5jMsHfVxvPbCQm4Nf8e6gqKUn84pBjyW`.
- Registry collection, merkle tree, and tree shard exist. Verified for `DLeBsmxSNB1RmcPrGSWm5J5tPqXjAKWVfqkVpCpCZdqY`, `4RJP3AJ6NNoqjTCxjeJi2Erw3wwJJoHN3jpwUrSetJw5`, and `8dvswMfvXUBg2YZSNoNijYaQBRNKqzzhyDToZq1Day8E`.
- Registry devnet E2E registers a smoke node and verifies canonical Bubblegum asset id. Verified for node `AFemTR5oys2tkRqCoDveYhfJ5mfD7kVV1M1zzKmGGewS`.
- Staking config exists and uses a non-negative `unbonding_seconds`. Verified for config `6RkwczSpZFKtZrAsqdRLSocogEtWYShPJwHjGd5QzrN1`, mint `Hfvwx9F5NDzMCyywJZJsFVX83XaXnLNntCdk21h7Bmcy`, `unbondingSeconds=60`.
- Reputation config exists. Verified for config `A693FZpF9XVv1WNxNyFqAeLrutktnm61VRKSdC8VgkSm`.
- Distributor exists and points to the expected mint, vault, treasury, poster authority, and dispute authority. Verified for rewards distributor `2u9qG5qyLWZC81Jaxdp7X9QbcLfB8GsySxHc129PsCMA`, reward vault `B7KFYJhSqNCC7VrB3sFYFpeAuowxdYDQJQTCtLvD6s2e`.
- Governance config exists with expected quorum, threshold, voting period, execution delay, and min proposal stake. Verified for governance config `C9mThuc2TDheS5H3ucby1JRgKSVyGfBNUVewCTJgadiL`; M5 executed proposal `0`.
- Token distributor exists on the genesis mint. Verified for IDO distributor `28i3ZFXrV7WvTv1PaQshF6E8GpnTQccoxEbGaDGDGm3F`, vault `AsfNRobLfqHLD9McKbSaKXFBkKEr2ioQXD2rSHH7Nvfp`, `totalClaimed=4000000000`.

## 4. Mutable Devnet Rehearsal

- Register or refresh one devnet user-exit node.
- Start the home node with `scripts/run-node.sh`.
- Verify the node appears live in the cabinet/network.
- Start the control-plane/relay path used by users.
- Fund test users from devnet custody/test allocations or the transfer-faucet; do not expect mint-authority faucet on the fresh genesis mint.
- Provision a personal VPN link.
- Connect from a phone.
- Confirm exit IP is the user node IP first, not VPS.
- Stop the user node and confirm fallback to VPS.
- Deposit test `$WEFT` into Payment Escrow.
- Generate traffic, settle it with `pay_traffic_from_escrow`, and confirm escrow balance, reward vault, treasury, and burned supply deltas match the governed split.
- Withdraw unused escrow balance and confirm the escrow cannot be overdrawn.
- Submit the same settlement signature twice and verify duplicate settlement is rejected.

## 5. Rewards Rehearsal

- Submit at least one valid dual-signed receipt batch to `/receipts`.
- Confirm aggregator builds an epoch and serves `/proof`.
- Post epoch root to devnet.
- Wait for dispute window or use a devnet-short window.
- Claim reward from cabinet/CLI.
- Confirm disputed or duplicate receipts do not produce a claimable reward.

## 6. Manual UX Checks

- Wallet connect works in cabinet.
- Devnet-only keypair connect/faucet UI is visible only on devnet.
- VPN link copy works on desktop and mobile browser.
- Happ/V2Box/Hiddify import works.
- Error messages are understandable when no node is live, no balance exists, or settlement is pending.
