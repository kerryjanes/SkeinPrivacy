// M8 devnet smoke: prove the IDO/TGE token-distributor end to end on the LIVE program.
// Create a test mint, post a tiny allocation merkle root, fund the distributor vault, and
// claim — asserting the claimant receives 25% liquid at TGE and a 12-month vesting
// schedule (created via CPI into weft-vesting) holding the other 75%.
//
// Run: WEFT_KEYPAIR=… WEFT_RPC_URL=<helius> pnpm --filter @weft/genesis exec tsx src/m8smoke.ts

import {
  generateKeyPairSigner,
  getAddressEncoder,
  getProgramDerivedAddress,
  type Address,
} from '@solana/kit';
import { getCreateAccountInstruction } from '@solana-program/system';
import {
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstructionAsync,
  getInitializeMint2Instruction,
  getMintSize,
  getMintToInstruction,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import {
  fetchVestingSchedule,
  math,
  WEFT_VESTING_PROGRAM_ADDRESS,
  tokenDistributor,
} from '@weft/sdk';

import { connect, loadSigner, send, type Connection } from './rpc';

const addrEnc = getAddressEncoder();

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

async function main(): Promise<void> {
  const rpcUrl = process.env.WEFT_RPC_URL ?? 'https://api.devnet.solana.com';
  const wsUrl = process.env.WEFT_RPC_WS ?? rpcUrl.replace(/^http/, 'ws').replace('8899', '8900');
  const keypairPath = process.env.WEFT_KEYPAIR ?? `${process.env.HOME}/.config/solana/id.json`;
  const conn = connect(rpcUrl, wsUrl);
  const deployer = await loadSigner(keypairPath);
  console.log(`[m8] claimant ${deployer.address}`);

  // 1. A test IDO mint (deployer = mint authority).
  const mint = await generateKeyPairSigner();
  const space = BigInt(getMintSize());
  const rent = await conn.rpc.getMinimumBalanceForRentExemption(space).send();
  const claimantAta = (
    await findAssociatedTokenPda({
      owner: deployer.address,
      mint: mint.address,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
    })
  )[0];
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
      decimals: 9,
      mintAuthority: deployer.address,
      freezeAuthority: null,
    }),
    await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: deployer,
      owner: deployer.address,
      mint: mint.address,
    }),
  ]);
  console.log(`[m8] IDO mint ${mint.address}`);

  // 2. A 1-leaf allocation tree for the claimant (root = the single leaf).
  const [distributor] = await tokenDistributor.findDistributorPda();
  const allocation = 4_000_000_000n; // 4 WEFT
  const leaf = math.hashAllocationLeaf(
    addrEnc.encode(distributor) as Uint8Array,
    addrEnc.encode(deployer.address) as Uint8Array,
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
        mint: mint.address,
        merkleRoot: root,
        tgeTs,
        tgeBps: 2_500,
        vestingDuration,
        totalAllocation: allocation,
      }),
    ]);
    console.log('[m8] distributor initialized');
  } else if (existing.data.mint !== mint.address) {
    throw new Error('distributor already initialized with a different mint; reset devnet state');
  }

  // 4. Fund the distributor vault with the full allocation.
  const [vault] = await tokenDistributor.findVaultPda();
  await send(conn, deployer, [
    getMintToInstruction({
      mint: mint.address,
      token: vault,
      mintAuthority: deployer,
      amount: allocation,
    }),
  ]);

  // 5. Claim → 25% TGE liquid + 75% vesting schedule via CPI.
  const [schedule] = await getProgramDerivedAddress({
    programAddress: WEFT_VESTING_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('schedule'), addrEnc.encode(deployer.address), u64le(0n)],
  });
  const [scheduleVault] = await getProgramDerivedAddress({
    programAddress: WEFT_VESTING_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('vault'), addrEnc.encode(schedule)],
  });

  const before = await tokenAmount(conn, claimantAta);
  await send(conn, deployer, [
    await tokenDistributor.getClaimInstructionAsync({
      claimant: deployer,
      mint: mint.address,
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
