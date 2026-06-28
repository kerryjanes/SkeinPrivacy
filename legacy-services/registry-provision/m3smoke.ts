// M3 devnet smoke: wire staking/reputation authorities into the live registry,
// stake + submit reputation against the real M2-registered node (node 1), and
// verify both mirror into NodeState (and surface in the indexer directory).

import { address, getProgramDerivedAddress, type Address } from '@solana/kit';
import {
  getCreateAssociatedTokenIdempotentInstructionAsync,
  findAssociatedTokenPda,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import { nodeRegistry, staking, reputation } from '@weft/sdk';
import { readFileSync } from 'node:fs';

import { loadEnv, nodePda, registryPda } from './config';
import { connect, loadSigner, send } from './kit';

const NODE_ID = 1n;
const STAKE_AMOUNT = 30_000n;
const GENESIS_MANIFEST = new URL('../../genesis/manifests/devnet.json', import.meta.url).pathname;

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
function loadGenesisMint(): Address {
  const mint = process.env.WEFT_MINT ?? JSON.parse(readFileSync(GENESIS_MANIFEST, 'utf8')).weftMint;
  return address(mint);
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

  // 2. Use the fresh devnet genesis mint. This keeps staking/reputation on the
  // same token that genesis, settlement, cabinet, and rewards use.
  const mint = loadGenesisMint();
  const opAta = await ata(deployer.address, mint);
  const treasury = opAta; // devnet rehearsal keeps custody under the deployer wallet.
  await send(conn, deployer, [
    await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: deployer,
      owner: deployer.address,
      mint,
    }),
  ]);
  console.log(`[m3] genesis mint ${mint}`);

  // 3. Initialize staking + reputation configs.
  const [stakingConfig] = await getProgramDerivedAddress({
    programAddress: staking.STAKING_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('staking_config')],
  });
  const [repConfig] = await getProgramDerivedAddress({
    programAddress: reputation.REPUTATION_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('reputation_config')],
  });
  const stakingMaybe = await staking.fetchMaybeStakingConfig(conn.rpc, stakingConfig);
  if (!stakingMaybe.exists) {
    await send(conn, deployer, [
      await staking.getInitializeConfigInstruction({
        authority: deployer,
        mint,
        treasury,
        slashAuthority: deployer.address,
        config: stakingConfig,
        unbondingSeconds: 60n,
      }),
    ]);
    console.log('[m3] staking config initialized');
  } else if (stakingMaybe.data.mint !== mint) {
    throw new Error(`staking mint ${stakingMaybe.data.mint} != genesis mint ${mint}`);
  }
  const repMaybe = await reputation.fetchMaybeReputationConfig(conn.rpc, repConfig);
  if (!repMaybe.exists) {
    await send(conn, deployer, [
      await reputation.getInitializeConfigInstruction({
        authority: deployer,
        oracle: deployer.address,
        config: repConfig,
      }),
    ]);
    console.log('[m3] reputation config initialized');
  }

  // 4. Stake against the real node → mirror into NodeState.stake_amount.
  const [position] = await staking.findPositionPda({ operator: deployer.address, nodeId: NODE_ID });
  const posMaybe = await staking.fetchMaybeStakePosition(conn.rpc, position);
  const currentStake = posMaybe.exists ? posMaybe.data.amount : 0n;
  if (currentStake < STAKE_AMOUNT) {
    await send(conn, deployer, [
      await staking.getStakeInstructionAsync({
        operator: deployer,
        operatorTokenAccount: opAta,
        mint,
        registry,
        node,
        nodeId: NODE_ID,
        amount: STAKE_AMOUNT - currentStake,
        lockDuration: 0n,
      }),
    ]);
  }

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
