#!/usr/bin/env -S tsx
import { address } from '@solana/kit';
import { weft } from '@weft/sdk';

import { connect, loadSigner, send } from './kit';
import { loadEnv } from './config';

const env = loadEnv();
const rewardMint = process.env.WEFT_MINT;
const treasury = process.env.WEFT_TREASURY_TOKEN_ACCOUNT;
const posterAuthority = process.env.WEFT_POSTER_AUTHORITY;
const disputeAuthority = process.env.WEFT_DISPUTE_AUTHORITY;

if (!rewardMint) throw new Error('WEFT_MINT is required');
if (!treasury) throw new Error('WEFT_TREASURY_TOKEN_ACCOUNT is required');
if (!posterAuthority) throw new Error('WEFT_POSTER_AUTHORITY is required');
if (!disputeAuthority) throw new Error('WEFT_DISPUTE_AUTHORITY is required');

const unbondingSeconds = BigInt(process.env.WEFT_UNBONDING_SECONDS ?? 7 * 24 * 60 * 60);
const disputeWindowSeconds = BigInt(process.env.WEFT_DISPUTE_WINDOW_SECONDS ?? 10 * 60);
const clawbackWindowSeconds = BigInt(process.env.WEFT_CLAWBACK_WINDOW_SECONDS ?? 24 * 60 * 60);

const conn = connect(env.rpcUrl, env.wsUrl);
const authority = await loadSigner(env.keypairPath);

const [registry] = await weft.findRegistryPda();
const existing = await conn.rpc.getAccountInfo(registry, { encoding: 'base64' }).send();
if (existing.value) {
  console.log(`Weft core already initialized on ${env.cluster}: ${registry}`);
  process.exit(0);
}

const ix = await weft.getInitializeCoreInstructionAsync({
  authority,
  rewardMint: address(rewardMint),
  posterAuthority: address(posterAuthority),
  disputeAuthority: address(disputeAuthority),
  treasury: address(treasury),
  unbondingSeconds,
  disputeWindowSeconds,
  clawbackWindowSeconds,
});

const signature = await send(conn, authority, [ix]);
const [stakingConfig] = await weft.findStakingConfigPda();
const [distributor] = await weft.findDistributorPda();
const [rewardVault] = await weft.findRewardVaultPda();

console.log(
  JSON.stringify(
    {
      cluster: env.cluster,
      program: weft.WEFT_PROGRAM_ADDRESS,
      signature,
      registry,
      stakingConfig,
      distributor,
      rewardVault,
    },
    null,
    2,
  ),
);
