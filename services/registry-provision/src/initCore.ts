#!/usr/bin/env -S tsx
import { address } from '@solana/kit';
import {
  TOKEN_PROGRAM_ADDRESS,
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstruction,
} from '@solana-program/token';
import { weft } from '@weft/sdk';

import { connect, loadSigner, send } from './kit';
import { loadEnv } from './config';

const env = loadEnv();
const rewardMint = process.env.WEFT_MINT;
const posterAuthority = process.env.WEFT_POSTER_AUTHORITY;
const disputeAuthority = process.env.WEFT_DISPUTE_AUTHORITY;

if (!rewardMint) throw new Error('WEFT_MINT is required');
if (!posterAuthority) throw new Error('WEFT_POSTER_AUTHORITY is required');
if (!disputeAuthority) throw new Error('WEFT_DISPUTE_AUTHORITY is required');

const unbondingSeconds = BigInt(process.env.WEFT_UNBONDING_SECONDS ?? 7 * 24 * 60 * 60);
const disputeWindowSeconds = BigInt(process.env.WEFT_DISPUTE_WINDOW_SECONDS ?? 10 * 60);
const clawbackWindowSeconds = BigInt(process.env.WEFT_CLAWBACK_WINDOW_SECONDS ?? 24 * 60 * 60);

const conn = connect(env.rpcUrl, env.wsUrl);
const authority = await loadSigner(env.keypairPath);
const mint = address(rewardMint);

// Treasury owner defaults to the deploy authority; its ATA for the reward mint is
// the on-chain treasury the escrow settlement flows into. Override the owner with
// WEFT_TREASURY_OWNER, or pin an exact account with WEFT_TREASURY_TOKEN_ACCOUNT.
const treasuryOwner = address(process.env.WEFT_TREASURY_OWNER ?? authority.address);

const [registry] = await weft.findRegistryPda();
const existing = await conn.rpc.getAccountInfo(registry, { encoding: 'base64' }).send();
if (existing.value) {
  console.log(`Weft core already initialized on ${env.cluster}: ${registry}`);
  process.exit(0);
}

// Guard: the reward mint must be a real, classic-SPL mint. The entire flow (treasury
// ATA, reward vault, direct node payouts, cabinet) derives token accounts with the
// classic Token program and initialize_core defaults token_program to it — a Token-2022
// mint would produce wrong ATAs / fail init. Abort clearly before any state is created.
const CLASSIC_TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';
const mintInfo = await conn.rpc.getAccountInfo(mint, { encoding: 'base64' }).send();
if (!mintInfo.value) {
  throw new Error(`WEFT_MINT ${rewardMint} is not an account on ${env.cluster}`);
}
if (mintInfo.value.owner !== CLASSIC_TOKEN_PROGRAM) {
  throw new Error(
    `WEFT_MINT ${rewardMint} is owned by ${mintInfo.value.owner}, not the SPL Token ` +
      `program (${CLASSIC_TOKEN_PROGRAM}). This launch path is classic-SPL-only (a Token-2022 ` +
      `mint would need code changes). Aborting before any state is created.`,
  );
}
const mintBytes = Buffer.from(mintInfo.value.data[0], 'base64');
if (mintBytes.length < 82) {
  throw new Error(
    `WEFT_MINT ${rewardMint} is not a valid SPL mint on ${env.cluster} (data too short)`,
  );
}
console.log(`[init] reward mint ${rewardMint}: owner=SPL Token, decimals=${mintBytes[44]}`);

// Resolve the treasury token account (explicit override, else the owner's ATA) and
// create it idempotently so initialize_core can load it as an existing TokenAccount.
let treasury = process.env.WEFT_TREASURY_TOKEN_ACCOUNT
  ? address(process.env.WEFT_TREASURY_TOKEN_ACCOUNT)
  : (
      await findAssociatedTokenPda({
        owner: treasuryOwner,
        mint,
        tokenProgram: TOKEN_PROGRAM_ADDRESS,
      })
    )[0];

const treasuryInfo = await conn.rpc.getAccountInfo(treasury, { encoding: 'base64' }).send();
if (!treasuryInfo.value) {
  const createAta = getCreateAssociatedTokenIdempotentInstruction({
    payer: authority,
    ata: treasury,
    owner: treasuryOwner,
    mint,
    tokenProgram: TOKEN_PROGRAM_ADDRESS,
  });
  const ataSig = await send(conn, authority, [createAta]);
  console.log(`[init] treasury ATA ${treasury} created for owner ${treasuryOwner} (${ataSig})`);
} else {
  console.log(`[init] treasury ATA ${treasury} already exists (owner ${treasuryOwner})`);
}

const ix = await weft.getInitializeCoreInstructionAsync({
  authority,
  rewardMint: mint,
  posterAuthority: address(posterAuthority),
  disputeAuthority: address(disputeAuthority),
  treasury,
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
      rewardMint: rewardMint,
      treasury,
      posterAuthority,
      disputeAuthority,
    },
    null,
    2,
  ),
);
