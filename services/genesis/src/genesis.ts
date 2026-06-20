// $WEFT genesis orchestrator: create the mint, mint the full fixed supply,
// route the six SPEC allocation buckets (3 liquid custody vaults + 4 vesting
// schedules), then retire the mint authority. Writes a deployment manifest.

import { getCreateAccountInstruction } from '@solana-program/system';
import {
  AuthorityType,
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstructionAsync,
  getInitializeMint2Instruction,
  getMintSize,
  getMintToInstruction,
  getSetAuthorityInstruction,
  getTransferCheckedInstruction,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import {
  findSchedulePda,
  findVaultPda,
  getCreateScheduleInstructionAsync,
  WEFT_VESTING_PROGRAM_ADDRESS,
} from '@weft/sdk';
import { generateKeyPairSigner, type Address } from '@solana/kit';

import {
  AMOUNTS,
  assertConservation,
  DECIMALS,
  ONE_WEFT,
  SCHEDULES,
  TOTAL_SUPPLY,
  type GenesisEnv,
} from './config';
import { loadManifest, saveManifest, type Manifest } from './manifest';
import { connect, loadSigner, send, type Connection } from './rpc';

export interface OwnerOverrides {
  treasury?: Address;
  emissions?: Address;
  /** Owner of the full 150M IDO custody bucket (funds the token-distributor vault). */
  ido?: Address;
  scheduleOwners?: Record<string, { beneficiary: Address; authority: Address }>;
}

const CUSTODY_KEYS = ['treasury', 'emissions', 'ido'] as const;
type CustodyKey = (typeof CUSTODY_KEYS)[number];

const CUSTODY_AMOUNT: Record<CustodyKey, bigint> = {
  treasury: AMOUNTS.treasury,
  emissions: AMOUNTS.nodeEmissions,
  // The whole IDO bucket (TGE + vesting); the distributor splits 25/75 per claimant.
  ido: AMOUNTS.idoTge + AMOUNTS.idoLinear,
};

async function ataFor(owner: Address, mint: Address): Promise<Address> {
  const [ata] = await findAssociatedTokenPda({ owner, mint, tokenProgram: TOKEN_PROGRAM_ADDRESS });
  return ata;
}

async function tokenBalance(conn: Connection, ata: Address): Promise<bigint> {
  const { value } = await conn.rpc.getTokenAccountBalance(ata).send();
  return BigInt(value.amount);
}

export async function runGenesis(
  env: GenesisEnv,
  overrides: OwnerOverrides = {},
): Promise<Manifest> {
  assertConservation();

  const existing = loadManifest(env.cluster);
  if (existing?.complete) {
    console.log(
      `[genesis] ${env.cluster} already complete (mint ${existing.weftMint}); skipping.`,
    );
    return existing;
  }

  const conn = connect(env.rpcUrl, env.wsUrl);
  const deployer = await loadSigner(env.keypairPath);
  console.log(`[genesis] deployer ${deployer.address} on ${env.cluster}`);

  // 1. Create the mint (decimals 9, freeze authority null, mint authority = deployer).
  const mint = await generateKeyPairSigner();
  const space = BigInt(getMintSize());
  const rent = await conn.rpc.getMinimumBalanceForRentExemption(space).send();
  await send(conn, deployer, [
    getCreateAccountInstruction({
      payer: deployer,
      newAccount: mint,
      lamports: rent,
      space,
      programAddress: TOKEN_PROGRAM_ADDRESS,
    }),
    getInitializeMint2Instruction({
      mint: mint.address,
      decimals: DECIMALS,
      mintAuthority: deployer.address,
      freezeAuthority: null,
    }),
  ]);
  console.log(`[genesis] mint ${mint.address}`);

  // 2. Mint the full fixed supply into the deployer's working ATA.
  const workingAta = await ataFor(deployer.address, mint.address);
  await send(conn, deployer, [
    await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: deployer,
      owner: deployer.address,
      mint: mint.address,
    }),
    getMintToInstruction({
      mint: mint.address,
      token: workingAta,
      mintAuthority: deployer,
      amount: TOTAL_SUPPLY,
    }),
  ]);
  console.log(`[genesis] minted ${TOTAL_SUPPLY / ONE_WEFT} WEFT to working ATA`);

  // 3. Route the three liquid custody buckets to their owner ATAs.
  const custody = {} as Manifest['custody'];
  for (const key of CUSTODY_KEYS) {
    const owner = overrides[key] ?? (await generateKeyPairSigner()).address;
    const ata = await ataFor(owner, mint.address);
    const amount = CUSTODY_AMOUNT[key];
    await send(conn, deployer, [
      await getCreateAssociatedTokenIdempotentInstructionAsync({
        payer: deployer,
        owner,
        mint: mint.address,
      }),
      getTransferCheckedInstruction({
        source: workingAta,
        mint: mint.address,
        destination: ata,
        authority: deployer,
        amount,
        decimals: DECIMALS,
      }),
    ]);
    custody[key] = { owner, ata, amount: amount.toString() };
    console.log(`[genesis] ${key}: ${amount / ONE_WEFT} WEFT → ${ata}`);
  }

  // 4. Create + fund the four vesting schedules from the working ATA.
  const schedules: Manifest['schedules'] = {};
  for (const s of SCHEDULES) {
    const so = overrides.scheduleOwners?.[s.key];
    const beneficiary = so?.beneficiary ?? (await generateKeyPairSigner()).address;
    const authority = so?.authority ?? deployer.address;
    const ix = await getCreateScheduleInstructionAsync({
      funder: deployer,
      beneficiary,
      authority,
      mint: mint.address,
      funderTokenAccount: workingAta,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
      scheduleId: s.scheduleId,
      totalAmount: s.amount,
      cliffUnlockAmount: 0n,
      startTs: env.tgeTimestamp,
      cliffDuration: s.cliff,
      duration: s.duration,
      revocable: s.revocable,
    });
    await send(conn, deployer, [ix]);
    const [schedule] = await findSchedulePda({ beneficiary, scheduleId: s.scheduleId });
    const [vault] = await findVaultPda({ schedule });
    schedules[s.key] = {
      schedule,
      vault,
      beneficiary,
      authority,
      amount: s.amount.toString(),
    };
    console.log(`[genesis] schedule ${s.key}: ${s.amount / ONE_WEFT} WEFT → vault ${vault}`);
  }

  // 5. The working ATA must be fully drained before retiring authority.
  const remaining = await tokenBalance(conn, workingAta);
  if (remaining !== 0n) {
    throw new Error(
      `working ATA still holds ${remaining} base units; aborting before authority retirement`,
    );
  }

  // 6. Retire the mint authority — supply is now provably fixed.
  await send(conn, deployer, [
    getSetAuthorityInstruction({
      owned: mint.address,
      owner: deployer,
      authorityType: AuthorityType.MintTokens,
      newAuthority: null,
    }),
  ]);
  console.log('[genesis] mint authority retired (fixed supply)');

  const manifest: Manifest = {
    cluster: env.cluster,
    complete: true,
    tgeTimestamp: env.tgeTimestamp.toString(),
    weftMint: mint.address,
    mintAuthorityRetired: true,
    vestingProgramId: WEFT_VESTING_PROGRAM_ADDRESS,
    custody,
    schedules,
  };
  saveManifest(manifest);
  return manifest;
}
