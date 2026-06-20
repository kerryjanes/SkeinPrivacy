// M3 devnet smoke: wire staking/reputation authorities into the live registry,
// stake + submit reputation against the real M2-registered node (node 1), and
// verify both mirror into NodeState (and surface in the indexer directory).

import {
  address,
  generateKeyPairSigner,
  getProgramDerivedAddress,
  type Address,
} from '@solana/kit';
import { getCreateAccountInstruction } from '@solana-program/system';
import {
  getCreateAssociatedTokenIdempotentInstructionAsync,
  getInitializeMint2Instruction,
  getMintSize,
  getMintToInstruction,
  findAssociatedTokenPda,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import { nodeRegistry, staking, reputation } from '@weft/sdk';

import { loadEnv, nodePda, registryPda } from './config';
import { connect, loadSigner, send } from './kit';

const NODE_ID = 1n;
const STAKE_AMOUNT = 30_000n;

async function authorityPda(program: Address): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: program,
    seeds: [new TextEncoder().encode('authority')],
  });
  return pda;
}
async function ata(owner: Address, mint: Address): Promise<Address> {
  const [a] = await findAssociatedTokenPda({ owner, mint, tokenProgram: TOKEN_PROGRAM_ADDRESS });
  return a;
}

async function main() {
  const env = loadEnv({ cluster: 'devnet' });
  const conn = connect(env.rpcUrl, env.wsUrl);
  const deployer = await loadSigner(env.keypairPath);
  const registry = await registryPda();
  const node = await nodePda(deployer.address, NODE_ID);
  console.log(`[m3] deployer ${deployer.address}; node ${node}`);

  // 1. Point the registry's metric writers at the two programs' authority PDAs.
  const stakingAuth = await authorityPda(staking.STAKING_PROGRAM_ADDRESS);
  const repAuth = await authorityPda(reputation.REPUTATION_PROGRAM_ADDRESS);
  await send(conn, deployer, [
    await nodeRegistry.getSetMetricsAuthoritiesInstructionAsync({
      authority: deployer,
      reputationAuthority: repAuth,
      stakingAuthority: stakingAuth,
    }),
  ]);
  console.log('[m3] set_metrics_authorities done');

  // 2. A test mint (deployer = mint authority) + funded operator ATA + treasury.
  const mint = await generateKeyPairSigner();
  const space = BigInt(getMintSize());
  const rent = await conn.rpc.getMinimumBalanceForRentExemption(space).send();
  const opAta = await ata(deployer.address, mint.address);
  const treasury = await ata(deployer.address, mint.address); // same owner; only needs to exist
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
    getMintToInstruction({
      mint: mint.address,
      token: opAta,
      mintAuthority: deployer,
      amount: 1_000_000n,
    }),
  ]);
  console.log(`[m3] test mint ${mint.address}`);

  // 3. Initialize staking + reputation configs.
  const [stakingConfig] = await getProgramDerivedAddress({
    programAddress: staking.STAKING_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('staking_config')],
  });
  const [repConfig] = await getProgramDerivedAddress({
    programAddress: reputation.REPUTATION_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('reputation_config')],
  });
  await send(conn, deployer, [
    await staking.getInitializeConfigInstruction({
      authority: deployer,
      mint: mint.address,
      treasury,
      slashAuthority: deployer.address,
      config: stakingConfig,
      unbondingSeconds: 60n,
    }),
  ]);
  await send(conn, deployer, [
    await reputation.getInitializeConfigInstruction({
      authority: deployer,
      oracle: deployer.address,
      config: repConfig,
    }),
  ]);
  console.log('[m3] configs initialized');

  // 4. Stake against the real node → mirror into NodeState.stake_amount.
  await send(conn, deployer, [
    await staking.getStakeInstructionAsync({
      operator: deployer,
      operatorTokenAccount: opAta,
      mint: mint.address,
      registry,
      node,
      nodeId: NODE_ID,
      amount: STAKE_AMOUNT,
      lockDuration: 0n,
    }),
  ]);

  // 5. Submit reputation metrics → mirror into NodeState.reputation.
  await send(conn, deployer, [
    await reputation.getUpdateMetricsInstructionAsync({
      oracle: deployer,
      operator: deployer.address,
      registry,
      node,
      nodeId: NODE_ID,
      uptimeBps: 9_500,
      speedBps: 9_000,
      reviewBps: 8_800,
    }),
  ]);

  // 6. Verify the mirror on the authoritative NodeState.
  const ns = await nodeRegistry.fetchNodeState(conn.rpc, node);
  console.log(`[m3] NodeState.stake_amount = ${ns.data.stakeAmount}`);
  console.log(`[m3] NodeState.reputation   = ${ns.data.reputation} bps`);
  if (ns.data.stakeAmount !== STAKE_AMOUNT)
    throw new Error(`stake mirror mismatch: ${ns.data.stakeAmount}`);
  if (ns.data.reputation < 5000 || ns.data.reputation > 20000)
    throw new Error(`reputation out of range: ${ns.data.reputation}`);
  console.log('[m3] ✅ stake + reputation mirrored into NodeState on devnet');
}

main().catch((e) => {
  console.error('[m3] failed:', e);
  process.exit(1);
});
