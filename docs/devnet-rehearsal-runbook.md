# Devnet Rehearsal Runbook

Use this runbook before mainnet. The rehearsal must use one coherent devnet mint across genesis, staking, settlement, control-plane, cabinet, and rewards.

## Current Devnet Status

Previous public devnet state was mixed and was archived before the fresh rehearsal:

- Genesis manifest mint: `8AYQEuGHXXwndyfLCY4quyNoMxTPxzh2CJv6DwpDaC8i`
- Staking config mint observed on devnet: `8ARbNvXE3C2VJs6n3ypuRRpVoDA6Z1CL8dxSiULUf4Qb`
- Settlement distributor reward mint observed on devnet: `H6WerhDCoqXapMy9y1fSYpbSHf9DcJEKjeLcfC1txtPJ`

Because staking and settlement use singleton PDAs and do not expose a mint-rotation instruction,
do not reuse that mixed state as a mainnet rehearsal. The active rehearsal now uses fresh devnet
program IDs, fresh genesis, and fresh registry provision.

## Fresh Devnet Rehearsal

Prepared branch: `rehearsal/devnet-mainnet-flow`.

Prepared devnet program IDs for this rehearsal:

- governance: `8uywvvcGdANC1WM7g1iuEq3crjBwhy5uP5UReKb3xNUE`
- node_registry: `GxhrTKKPybHZPv2MsaLovzKaq9Pq8jHmjNyRMrKZY6aH`
- reputation: `6E9RfpzBbyshMBrbnSHne8wzjMqQgYPFMDdP2Xra8SEZ`
- rewards_settlement: `BecoJTYnDmFTTde84LwSBfYqEq7RN4qdysqnep2Gv9GU`
- weft_vesting: `Aa8aMMVmxcA5CKQAJQ3N3EmFtKYTx79hEEYjFYzBCDjb`
- staking: `CvvsFn1SFV2R19WhXmqYyMjbKUWADcej93F6DadHv3yU`
- token_distributor: `89wHYTG6deQUn4Rsbkcw2gG2sVSk8g9jTezP9qxG66Dn`

Rent-optimized build baseline after removing production cross-program crate dependencies and
Anchor instruction-name logs:

| Program | Bytes | Rent-exempt minimum |
| --- | ---: | ---: |
| governance | 280,328 | 1.95197376 SOL |
| node_registry | 264,152 | 1.8393888 SOL |
| reputation | 196,360 | 1.36755648 SOL |
| rewards_settlement | 369,864 | 2.57514432 SOL |
| weft_vesting | 224,072 | 1.560432 SOL |
| staking | 262,528 | 1.82808576 SOL |
| token_distributor | 287,040 | 1.99868928 SOL |

Total programdata rent baseline: `13.1212704 SOL` before transaction fees and any non-program
accounts. Previous baseline was `13.5146496 SOL`; saved `0.3933792 SOL`.

Fresh devnet genesis:

- mint: `Hfvwx9F5NDzMCyywJZJsFVX83XaXnLNntCdk21h7Bmcy`
- mint authority: retired
- total supply: `1000000000000000000`
- decimals: `9`
- manifest: `services/genesis/manifests/devnet.json`

Fresh devnet registry provision:

- registry PDA: `6tw8x8sm18fz5jMsHfVxvPbCQm4Nf8e6gqKUn84pBjyW`
- collection: `DLeBsmxSNB1RmcPrGSWm5J5tPqXjAKWVfqkVpCpCZdqY`
- merkle tree: `4RJP3AJ6NNoqjTCxjeJi2Erw3wwJJoHN3jpwUrSetJw5`
- tree shard: `8dvswMfvXUBg2YZSNoNijYaQBRNKqzzhyDToZq1Day8E`
- max depth: `14`
- manifest: `services/registry-provision/manifests/devnet.json`
- smoke node PDA: `AFemTR5oys2tkRqCoDveYhfJ5mfD7kVV1M1zzKmGGewS`
- smoke node asset id: `AZDdrqcp7ZBm5BJDbWmmNuyY1MVJPaFNx5qY4sHsCxuD`

Verified on devnet:

- all seven fresh program accounts are executable and have the expected data lengths.
- genesis rerun skips when the manifest is complete.
- registry provision rerun skips when the manifest is complete.
- registry account decodes with authority `2m5CoAk7ioZJbRYqHV9PJMNZN2gwpTPKQXR4GKyVifL7`, matching collection/tree, and `paused=false`.
- `@weft/registry-provision` devnet E2E registers a node and verifies the canonical Bubblegum asset id.

Fresh devnet singleton configuration on the same mint `Hfvwx9F5NDzMCyywJZJsFVX83XaXnLNntCdk21h7Bmcy`:

- staking config: `6RkwczSpZFKtZrAsqdRLSocogEtWYShPJwHjGd5QzrN1`
- staking treasury: `3YAnZZPzy7545f8PXwK8AzpKbHTGJonDPQExVrVJe371`
- staking slash authority: `6kTtDGxH2BcWFgViwbCjDwycd1Nep27tMC75zREzumQ6` (settlement authority PDA after M4)
- reputation config: `A693FZpF9XVv1WNxNyFqAeLrutktnm61VRKSdC8VgkSm`
- reputation oracle: `6kTtDGxH2BcWFgViwbCjDwycd1Nep27tMC75zREzumQ6` (settlement authority PDA after M4)
- rewards distributor: `2u9qG5qyLWZC81Jaxdp7X9QbcLfB8GsySxHc129PsCMA`
- rewards vault: `B7KFYJhSqNCC7VrB3sFYFpeAuowxdYDQJQTCtLvD6s2e`
- governance config: `C9mThuc2TDheS5H3ucby1JRgKSVyGfBNUVewCTJgadiL`
- protocol config: `jxqq5y8mjgkHaZ1x5csVnZnrZ53DucBzx8923diX3hV`
- IDO distributor: `28i3ZFXrV7WvTv1PaQshF6E8GpnTQccoxEbGaDGDGm3F`
- IDO vault: `AsfNRobLfqHLD9McKbSaKXFBkKEr2ioQXD2rSHH7Nvfp`

Verified smoke flows:

- M3: staking/reputation initialized on the genesis mint; smoke node stake/reputation mirrored into NodeState.
- M5: governance + protocol config initialized; a proposal voted, finalized, timelocked, and executed.
- M4: settlement distributor initialized on the genesis mint; pay, fund, post, claim, dispute all landed. The CLI hit public devnet RPC `429` during confirmation after the dispute transaction landed; on-chain `ClaimStatus` for epoch `2` is `disputed=true`, stake was slashed, and reputation was penalized.
- M8: token-distributor initialized on the genesis mint; a separate persisted devnet claimant received 25% liquid and 75% vesting schedule.

1. Create a rehearsal branch.

```bash
git switch -c rehearsal/devnet-mainnet-flow
```

2. Generate new devnet program keypairs or choose a clean Anchor deployment keypair set.

3. Update all program IDs for the rehearsal branch:

- `Anchor.toml` `[programs.devnet]`
- every `declare_id!` in `programs/*/src/lib.rs`

4. Build and deploy to devnet.

```bash
NO_DNA=1 anchor build
anchor deploy --provider.cluster devnet
pnpm -r build
pnpm -F @weft/control-plane bundle
```

5. Run genesis on devnet with explicit owners. Devnet may use your deployer wallet as every owner for rehearsal, but mainnet must use multisig owners.

```bash
export WEFT_CLUSTER=devnet
export WEFT_RPC_URL=https://api.devnet.solana.com
export WEFT_TGE_TS=$(date +%s)
export WEFT_TREASURY_OWNER=<DEVNET_OWNER>
export WEFT_EMISSIONS_OWNER=<DEVNET_OWNER>
export WEFT_IDO_OWNER=<DEVNET_OWNER>
export WEFT_TEAM_BENEFICIARY=<DEVNET_OWNER>
export WEFT_TEAM_AUTHORITY=<DEVNET_OWNER>
export WEFT_ECOSYSTEM_BENEFICIARY=<DEVNET_OWNER>
export WEFT_ECOSYSTEM_AUTHORITY=<DEVNET_OWNER>
export WEFT_MARKETING_BENEFICIARY=<DEVNET_OWNER>
export WEFT_MARKETING_AUTHORITY=<DEVNET_OWNER>
pnpm -F @weft/genesis genesis
```

6. Provision the registry.

```bash
export WEFT_CLUSTER=devnet
export WEFT_RPC_URL=https://api.devnet.solana.com
pnpm -F @weft/registry-provision provision
```

7. Verify registry provisioning.

```bash
WEFT_CLUSTER=devnet \
WEFT_RPC_URL=https://api.devnet.solana.com \
WEFT_KEYPAIR=$HOME/.config/solana/id.json \
pnpm -F @weft/registry-provision exec vitest run \
  test/mainnet-config.test.ts test/registry.devnet.test.ts
```

8. Initialize staking and reputation using the same devnet mint from `services/genesis/manifests/devnet.json`.

```bash
WEFT_CLUSTER=devnet \
WEFT_RPC_URL=https://api.devnet.solana.com \
WEFT_KEYPAIR=$HOME/.config/solana/id.json \
pnpm -F @weft/registry-provision exec tsx src/m3smoke.ts
```

9. Initialize governance and run the DAO smoke.

```bash
WEFT_RPC_URL=https://api.devnet.solana.com \
WEFT_KEYPAIR=$HOME/.config/solana/id.json \
pnpm -F @weft/governance smoke
```

10. Initialize settlement/rewards and run the economic smoke.

```bash
WEFT_RPC_URL=https://api.devnet.solana.com \
WEFT_KEYPAIR=$HOME/.config/solana/id.json \
pnpm -F @weft/aggregator smoke
```

11. Initialize token distributor and run the IDO/TGE smoke.

```bash
WEFT_RPC_URL=https://api.devnet.solana.com \
WEFT_KEYPAIR=$HOME/.config/solana/id.json \
pnpm -F @weft/genesis m8smoke
```

12. Configure the node/control-plane rehearsal environment.

```bash
export WEFT_CLUSTER=devnet
export WEFT_RPC=https://api.devnet.solana.com
export WEFT_MINT=<DEVNET_GENESIS_MINT>
export WEFT_FAUCET_KEYPAIR=<DEVNET_MINT_AUTHORITY_KEYPAIR_IF_USED>
```

13. Configure cabinet devnet env.

```bash
export VITE_WEFT_CLUSTER=devnet
export VITE_WEFT_RPC_URL=https://api.devnet.solana.com
export VITE_WEFT_AGGREGATOR_URL=<DEVNET_AGGREGATOR_URL>
export VITE_WEFT_CONTROL_PLANE_URL=<DEVNET_CONTROL_PLANE_URL>
```

14. Run the full mutable rehearsal:

- faucet or fund wallet with devnet `$WEFT`
- provision VPN link
- connect from phone
- verify user-node exit IP
- stop user node and verify VPS fallback
- consume traffic
- run `pay_traffic`
- submit duplicate settlement and verify rejection
- submit receipts
- post epoch
- claim reward

## Read-only Commands For Any Devnet

```bash
pnpm -r build
pnpm -r test
pnpm -C ../weft-web/cabinet build
NO_DNA=1 anchor build
cargo test --workspace --lib --tests
```

## Manual Device Checks

These cannot be completed by automation on this machine:

- Import generated VLESS link on the phone.
- Confirm the phone sees the user node public IP.
- Stop the user node and confirm fallback to VPS.
- Confirm the cabinet UX is clear for empty balance, no live node, pending settlement, and claim states.
