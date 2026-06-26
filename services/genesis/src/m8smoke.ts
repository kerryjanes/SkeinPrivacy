// M8 devnet smoke: prove the IDO/TGE token-distributor end to end on the LIVE program.
// Use the fresh devnet genesis mint, post a tiny allocation merkle root, fund the
// distributor vault from the devnet IDO custody account, and claim — asserting
// the claimant receives 25% liquid at TGE and a 12-month vesting schedule
// (created via CPI into weft-vesting) holding the other 75%.
//
// Run: WEFT_KEYPAIR=… WEFT_RPC_URL=<helius> pnpm --filter @weft/genesis exec tsx src/m8smoke.ts

import {
  createKeyPairSignerFromBytes,
  generateKeyPairSigner,
  getAddressEncoder,
  getProgramDerivedAddress,
  lamports,
  writeKeyPairSigner,
  type Address,
} from '@solana/kit';
import { getTransferSolInstruction } from '@solana-program/system';
import {
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstructionAsync,
  getTransferCheckedInstruction,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import { existsSync, readFileSync } from 'node:fs';
import {
  fetchVestingSchedule,
  math,
  WEFT_VESTING_PROGRAM_ADDRESS,
  tokenDistributor,
} from '@weft/sdk';

import { connect, loadSigner, send, type Connection } from './rpc';

const addrEnc = getAddressEncoder();
const GENESIS_MANIFEST = new URL('../manifests/devnet.json', import.meta.url).pathname;
const CLAIMANT_FILE = new URL('../.m8-claimant.json', import.meta.url).pathname;

function u64le(v: bigint): Uint8Array {
  const out = new Uint8Array(8);
  let x = v;
  for (let i = 0; i < 8; i++) {
    out[i] = Number(x & 0xffn);
    x >>= 8n;
  }
  return out;
}

async function tokenAmount(conn: Connection, addr: Address): Promise<bigint> {
  const acc = await conn.rpc.getAccountInfo(addr, { encoding: 'base64' }).send();
  if (!acc.value) return 0n;
  const data = Buffer.from(acc.value.data[0], 'base64');
  let v = 0n;
  for (let i = 7; i >= 0; i--) v = (v << 8n) | BigInt(data[64 + i]);
  return v;
}

async function loadOrCreateClaimant() {
  if (existsSync(CLAIMANT_FILE)) {
    return createKeyPairSignerFromBytes(
      Uint8Array.from(JSON.parse(readFileSync(CLAIMANT_FILE, 'utf8')) as number[]),
    );
  }
  const signer = await generateKeyPairSigner(true);
  await writeKeyPairSigner(signer, CLAIMANT_FILE);
  return signer;
}

async function main(): Promise<void> {
  const rpcUrl = process.env.WEFT_RPC_URL ?? 'https://api.devnet.solana.com';
  const wsUrl = process.env.WEFT_RPC_WS ?? rpcUrl.replace(/^http/, 'ws').replace('8899', '8900');
  const keypairPath = process.env.WEFT_KEYPAIR ?? `${process.env.HOME}/.config/solana/id.json`;
  const conn = connect(rpcUrl, wsUrl);
  const deployer = await loadSigner(keypairPath);
  const genesis = JSON.parse(readFileSync(GENESIS_MANIFEST, 'utf8')) as {
    weftMint: Address;
    custody: { ido: { ata: Address } };
  };
  const mint = genesis.weftMint;
  const idoCustodyAta = genesis.custody.ido.ata;
  const claimant = await loadOrCreateClaimant();
  console.log(`[m8] authority ${deployer.address}; claimant ${claimant.address}; mint ${mint}`);

  // 1. Create the claimant ATA and fund claimant SOL for claim-created rent accounts.
  const claimantAta = (
    await findAssociatedTokenPda({
      owner: claimant.address,
      mint,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
    })
  )[0];
  await send(conn, deployer, [
    getTransferSolInstruction({
      source: deployer,
      destination: claimant.address,
      amount: lamports(30_000_000n),
    }),
    await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: deployer,
      owner: claimant.address,
      mint,
    }),
  ]);

  // 2. A 1-leaf allocation tree for the claimant (root = the single leaf).
  const [distributor] = await tokenDistributor.findDistributorPda();
  const allocation = 4_000_000_000n; // 4 WEFT
  const leaf = math.hashAllocationLeaf(
    addrEnc.encode(distributor) as Uint8Array,
    addrEnc.encode(claimant.address) as Uint8Array,
    allocation,
  );
  const root = math.merkleRoot([leaf]); // single leaf → root == leaf
  const proof: Uint8Array[] = math.merkleProof([leaf], 0); // empty proof

  // TGE in the past so claiming is open immediately; 12-month linear vesting.
  const now = BigInt(Math.floor(Date.now() / 1000));
  const tgeTs = now - 60n;
  const vestingDuration = 365n * 24n * 3600n;

  // 3. Initialize the distributor (idempotent — skip if already present).
  const existing = await tokenDistributor.fetchMaybeIdoDistributor(conn.rpc, distributor);
  if (!existing.exists) {
    await send(conn, deployer, [
      await tokenDistributor.getInitializeIdoInstructionAsync({
        authority: deployer,
        mint,
        merkleRoot: root,
        tgeTs,
        tgeBps: 2_500,
        vestingDuration,
        totalAllocation: allocation,
      }),
    ]);
    console.log('[m8] distributor initialized');
  } else if (existing.data.mint !== mint) {
    throw new Error(`distributor mint ${existing.data.mint} != genesis mint ${mint}`);
  }

  // 4. Fund the distributor vault with the full allocation.
  const [vault] = await tokenDistributor.findVaultPda();
  const vaultBalance = await tokenAmount(conn, vault);
  if (vaultBalance < allocation) {
    await send(conn, deployer, [
      getTransferCheckedInstruction({
        source: idoCustodyAta,
        mint,
        destination: vault,
        authority: deployer,
        amount: allocation - vaultBalance,
        decimals: 9,
      }),
    ]);
  }

  // 5. Claim → 25% TGE liquid + 75% vesting schedule via CPI.
  const [schedule] = await getProgramDerivedAddress({
    programAddress: WEFT_VESTING_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('schedule'), addrEnc.encode(claimant.address), u64le(0n)],
  });
  const [scheduleVault] = await getProgramDerivedAddress({
    programAddress: WEFT_VESTING_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('vault'), addrEnc.encode(schedule)],
  });

  const before = await tokenAmount(conn, claimantAta);
  await send(conn, deployer, [
    await tokenDistributor.getClaimInstructionAsync({
      claimant,
      mint,
      vault,
      claimantTokenAccount: claimantAta,
      vestingProgram: WEFT_VESTING_PROGRAM_ADDRESS,
      schedule,
      scheduleVault,
      amount: allocation,
      proof,
    }),
  ]);

  const tge = (allocation * 2_500n) / 10_000n;
  const vest = allocation - tge;
  const credited = (await tokenAmount(conn, claimantAta)) - before;
  const vested = await tokenAmount(conn, scheduleVault);
  console.log(`[m8] TGE credited ${credited} (exp ${tge}); vesting vault ${vested} (exp ${vest})`);
  if (credited !== tge) throw new Error('TGE split mismatch');
  if (vested !== vest) throw new Error('vesting split mismatch');

  const sched = await fetchVestingSchedule(conn.rpc, schedule);
  if (sched.data.totalAmount !== vest) throw new Error('schedule total mismatch');
  if (sched.data.duration !== vestingDuration) throw new Error('schedule duration mismatch');
  console.log('[m8] ✅ IDO claim verified on devnet: 25% TGE liquid + 75% vesting schedule');
}

main().catch((e) => {
  console.error('[m8] failed:', e);
  process.exit(1);
});
