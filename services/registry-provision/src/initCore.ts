#!/usr/bin/env -S tsx
import { address } from '@solana/kit';
import {
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

// Guard: detect the reward mint's owning token program (classic SPL or Token-2022 —
// pump.fun now mints Token-2022) and use it everywhere. Token-2022 is accepted ONLY if
// it carries no economic-breaking extension (transfer fee, transfer hook, permanent
// delegate, default-frozen, interest, non-transferable, confidential) — those silently
// break settlement/burn/payout. Metadata extensions are fine. Abort before any state.
const CLASSIC_TOKEN_PROGRAM = address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const TOKEN_2022_PROGRAM = address('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb');
const mintInfo = await conn.rpc.getAccountInfo(mint, { encoding: 'base64' }).send();
if (!mintInfo.value) {
  throw new Error(`WEFT_MINT ${rewardMint} is not an account on ${env.cluster}`);
}
const mintOwner = mintInfo.value.owner;
if (mintOwner !== CLASSIC_TOKEN_PROGRAM && mintOwner !== TOKEN_2022_PROGRAM) {
  throw new Error(
    `WEFT_MINT ${rewardMint} is owned by ${mintOwner}, which is neither the SPL Token ` +
      `program nor Token-2022. Aborting before any state is created.`,
  );
}
const tokenProgram = mintOwner === TOKEN_2022_PROGRAM ? TOKEN_2022_PROGRAM : CLASSIC_TOKEN_PROGRAM;
const mintBytes = Buffer.from(mintInfo.value.data[0], 'base64');
if (mintBytes.length < 82) {
  throw new Error(
    `WEFT_MINT ${rewardMint} is not a valid SPL mint on ${env.cluster} (data too short)`,
  );
}
// Reward math (USER_PRICE_PER_GB, baseUnitScale, the 70/20/10 split) is calibrated for a mint with
// at least 6 decimals — pump.fun's standing default. A lower-decimal mint would distort every
// amount; refuse it before any state exists rather than settle wrong numbers later.
const mintDecimals = mintBytes[44];
if (mintDecimals < 6) {
  throw new Error(
    `WEFT_MINT ${rewardMint} has ${mintDecimals} decimals; Weft economics require >= 6. Aborting.`,
  );
}
if (tokenProgram === TOKEN_2022_PROGRAM) {
  const DANGEROUS = new Set([
    'transferFeeConfig',
    'transferHook',
    'permanentDelegate',
    'defaultAccountState',
    'confidentialTransferMint',
    'confidentialTransferFeeConfig',
    'interestBearingConfig',
    'nonTransferable',
    'pausable',
    'scaledUiAmountConfig',
  ]);
  const parsed = await conn.rpc.getAccountInfo(mint, { encoding: 'jsonParsed' }).send();
  const info = (parsed.value?.data as { parsed?: { info?: { extensions?: Array<{ extension: string }> } } })
    ?.parsed?.info;
  // Fail CLOSED: if the RPC returned no parsed extension list (base64 fallback, or no jsonParsed
  // support for Token-2022), we cannot prove the mint is extension-free — a fee/hook mint would
  // slip through and silently break the economics. Abort and demand a jsonParsed-capable RPC.
  if (!info || !Array.isArray(info.extensions)) {
    throw new Error(
      `Could not read Token-2022 extensions for ${rewardMint} — the RPC returned no jsonParsed ` +
        `data. Use a jsonParsed-capable RPC (e.g. Helius) and retry. Aborting before any state.`,
    );
  }
  const exts = info.extensions.map((e) => e.extension);
  const bad = exts.filter((e) => DANGEROUS.has(e));
  if (bad.length > 0) {
    throw new Error(
      `WEFT_MINT ${rewardMint} is a Token-2022 mint with unsupported extension(s): ${bad.join(', ')}. ` +
        `These break settlement/burn/payout math. Aborting before any state is created.`,
    );
  }
  console.log(
    `[init] reward mint ${rewardMint}: owner=Token-2022, decimals=${mintDecimals}, extensions=[${exts.join(', ')}]`,
  );
} else {
  console.log(`[init] reward mint ${rewardMint}: owner=SPL Token (classic), decimals=${mintDecimals}`);
}

// Resolve the treasury token account (explicit override, else the owner's ATA) and
// create it idempotently so initialize_core can load it as an existing TokenAccount.
let treasury = process.env.WEFT_TREASURY_TOKEN_ACCOUNT
  ? address(process.env.WEFT_TREASURY_TOKEN_ACCOUNT)
  : (
      await findAssociatedTokenPda({
        owner: treasuryOwner,
        mint,
        tokenProgram,
      })
    )[0];

const treasuryInfo = await conn.rpc.getAccountInfo(treasury, { encoding: 'base64' }).send();
if (!treasuryInfo.value) {
  const createAta = getCreateAssociatedTokenIdempotentInstruction({
    payer: authority,
    ata: treasury,
    owner: treasuryOwner,
    mint,
    tokenProgram,
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
  tokenProgram,
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
