// M4 devnet smoke: exercise the full settlement economic loop against the LIVE
// programs and the real node 1 (operated by the deployer, staked + scored in M3):
//   1. pay_traffic  → observe the 70/20/10 split + burn (supply drop)
//   2. fund_vault   → top up the reward pool
//   3. build an epoch from a synthetic dual-signed receipt for node 1, post it
//   4. claim with the served proof → operator ATA credited
//   5. re-point slash_authority/oracle → settlement PDA, dispute a second epoch's
//      leaf → live StakePosition slashed + reputation penalized (mirrored into
//      NodeState), and the disputed leaf becomes unclaimable.
//
// Reward token is a test mint (the deployer holds no $WEFT); the mechanism is
// token-agnostic. The mint keypair is persisted (gitignored) so re-runs reuse
// the singleton distributor. Run: WEFT_KEYPAIR=… pnpm --filter @weft/aggregator smoke

import { randomBytes } from 'node:crypto';
import { existsSync, writeFileSync } from 'node:fs';
import { ed25519 } from '@noble/curves/ed25519';
import {
  createKeyPairSignerFromBytes,
  getAddressDecoder,
  getProgramDerivedAddress,
  lamports,
  type Address,
  type KeyPairSigner,
} from '@solana/kit';
import { getCreateAccountInstruction, getTransferSolInstruction } from '@solana-program/system';
import {
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstructionAsync,
  getInitializeMint2Instruction,
  getMintSize,
  getMintToInstruction,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import { math, nodeRegistry, reputation, rewardsSettlement, staking } from '@weft/sdk';

import { connect, loadEd25519Seed, loadSigner, send, type Connection } from './kit';
import { buildEpoch } from './rewards';
import { signReceiptCore, type TrafficReceipt } from './receipts';
import { epochRange } from './epoch';
import { fetchNodeInfos, fetchNodeStateTolerant } from './nodes';

const NODE_ID = 1n;
const MINT_FILE = new URL('../.m4-reward-mint.json', import.meta.url).pathname;
const addrDec = getAddressDecoder();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

interface RawKeypair {
  seed: Uint8Array;
  address: Address;
  secret64: Uint8Array;
}
function makeRawKeypair(): RawKeypair {
  const seed = Uint8Array.from(randomBytes(32));
  const pub = ed25519.getPublicKey(seed);
  const secret64 = new Uint8Array(64);
  secret64.set(seed, 0);
  secret64.set(pub, 32);
  return { seed, address: addrDec.decode(pub), secret64 };
}

function makeReceipt(
  clientAddr: Address,
  operatorAddr: Address,
  clientSeed: Uint8Array,
  operatorSeed: Uint8Array,
  p: { nodeId: bigint; bytes: bigint; epoch: bigint; nonce: bigint },
): TrafficReceipt {
  const { start } = epochRange(p.epoch);
  const core = {
    client: clientAddr,
    operator: operatorAddr,
    nodeId: p.nodeId,
    bytes: p.bytes,
    windowStart: start,
    windowEnd: start + 1n,
    nonce: p.nonce,
  };
  return {
    ...core,
    clientSig: signReceiptCore(core, clientSeed),
    relaySig: signReceiptCore(core, operatorSeed),
  };
}

async function pda(program: Address, seed: string): Promise<Address> {
  const [p] = await getProgramDerivedAddress({
    programAddress: program,
    seeds: [new TextEncoder().encode(seed)],
  });
  return p;
}
async function ata(owner: Address, mint: Address): Promise<Address> {
  const [a] = await findAssociatedTokenPda({ owner, mint, tokenProgram: TOKEN_PROGRAM_ADDRESS });
  return a;
}
function leU64(data: Uint8Array, offset: number): bigint {
  let v = 0n;
  for (let i = 7; i >= 0; i--) v = (v << 8n) | BigInt(data[offset + i]);
  return v;
}
async function tokenAmount(conn: Connection, addr: Address): Promise<bigint> {
  const acc = await conn.rpc.getAccountInfo(addr, { encoding: 'base64' }).send();
  return acc.value ? leU64(Buffer.from(acc.value.data[0], 'base64'), 64) : 0n;
}
async function supplyOf(conn: Connection, addr: Address): Promise<bigint> {
  const acc = await conn.rpc.getAccountInfo(addr, { encoding: 'base64' }).send();
  return leU64(Buffer.from(acc.value!.data[0], 'base64'), 36);
}

async function loadOrCreateMint(): Promise<{ signer: KeyPairSigner; existed: boolean }> {
  if (existsSync(MINT_FILE)) return { signer: await loadSigner(MINT_FILE), existed: true };
  const kp = makeRawKeypair();
  writeFileSync(MINT_FILE, JSON.stringify(Array.from(kp.secret64)));
  return { signer: await createKeyPairSignerFromBytes(kp.secret64), existed: false };
}

async function main(): Promise<void> {
  const rpcUrl = process.env.WEFT_RPC_URL ?? 'https://api.devnet.solana.com';
  const wsUrl = process.env.WEFT_RPC_WS ?? rpcUrl.replace(/^http/, 'ws').replace('8899', '8900');
  const keypairPath = process.env.WEFT_KEYPAIR ?? `${process.env.HOME}/.config/solana/id.json`;

  const conn = connect(rpcUrl, wsUrl);
  const deployer = await loadSigner(keypairPath);
  const operatorSeed = loadEd25519Seed(keypairPath);
  console.log(`[m4] deployer/operator ${deployer.address}`);

  const [distributorPda] = await rewardsSettlement.findDistributorPda();
  const [rewardVaultPda] = await rewardsSettlement.findRewardVaultPda();
  const settlementAuth = await pda(
    rewardsSettlement.REWARDS_SETTLEMENT_PROGRAM_ADDRESS,
    'authority',
  );
  const [stakingConfig] = await staking.findConfigPda();
  const [stakingAuth] = await staking.findProgramAuthorityPda();
  const [repConfig] = await reputation.findConfigPda();
  const [repAuth] = await reputation.findProgramAuthorityPda();
  const [repState] = await reputation.findStatePda({ operator: deployer.address, nodeId: NODE_ID });
  const [position] = await staking.findPositionPda({ operator: deployer.address, nodeId: NODE_ID });
  const registry = await pda(nodeRegistry.NODE_REGISTRY_PROGRAM_ADDRESS, 'registry');
  const [node] = await nodeRegistry.findNodePda({ operator: deployer.address, nodeId: NODE_ID });

  const cfg = await staking.fetchStakingConfig(conn.rpc, stakingConfig);
  const stakeMint = cfg.data.mint;
  const stakeTreasury = cfg.data.treasury;
  const pos0 = await staking.fetchStakePosition(conn.rpc, position);
  console.log(`[m4] stake mint ${stakeMint}; position amount ${pos0.data.amount}`);

  const { signer: rewardMint, existed } = await loadOrCreateMint();
  const deployerRewardAta = await ata(deployer.address, rewardMint.address);
  const distMaybe = await rewardsSettlement.fetchMaybeDistributor(conn.rpc, distributorPda);

  if (!existed) {
    const space = BigInt(getMintSize());
    const rent = await conn.rpc.getMinimumBalanceForRentExemption(space).send();
    await send(conn, deployer, [
      getCreateAccountInstruction({
        payer: deployer,
        newAccount: rewardMint,
        lamports: rent,
        space,
        programAddress: TOKEN_PROGRAM_ADDRESS,
      }),
      getInitializeMint2Instruction({
        mint: rewardMint.address,
        decimals: 9,
        mintAuthority: deployer.address,
        freezeAuthority: null,
      }),
      await getCreateAssociatedTokenIdempotentInstructionAsync({
        payer: deployer,
        owner: deployer.address,
        mint: rewardMint.address,
      }),
    ]);
    console.log(`[m4] created reward mint ${rewardMint.address}`);
  }
  await send(conn, deployer, [
    getMintToInstruction({
      mint: rewardMint.address,
      token: deployerRewardAta,
      mintAuthority: deployer,
      amount: 5_000_000_000n,
    }),
  ]);

  if (!distMaybe.exists) {
    await send(conn, deployer, [
      await rewardsSettlement.getInitializeDistributorInstructionAsync({
        authority: deployer,
        rewardMint: rewardMint.address,
        posterAuthority: deployer.address,
        disputeAuthority: deployer.address,
        treasury: deployerRewardAta,
        disputeWindowSeconds: 1n,
        clawbackWindowSeconds: 2n,
      }),
    ]);
    console.log('[m4] distributor initialized');
  }
  const dist = await rewardsSettlement.fetchDistributor(conn.rpc, distributorPda);
  if (dist.data.rewardMint !== rewardMint.address)
    throw new Error(
      `distributor mint ${dist.data.rewardMint} != ${rewardMint.address}; delete ${MINT_FILE}`,
    );
  const treasury = dist.data.treasury;
  const baseEpoch = dist.data.currentEpoch;

  // pay_traffic needs a payer whose token account differs from the treasury
  // (Anchor forbids the same account appearing twice as mutable). Fund a fresh
  // payer with SOL + reward tokens for the demo.
  const payerKp = makeRawKeypair();
  const payerSigner = await createKeyPairSignerFromBytes(payerKp.secret64);
  const payerAta = await ata(payerKp.address, rewardMint.address);
  const payAmount = 200_000_000n;
  await send(conn, deployer, [
    getTransferSolInstruction({
      source: deployer,
      destination: payerKp.address,
      amount: lamports(20_000_000n),
    }),
  ]);
  await send(conn, deployer, [
    await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: deployer,
      owner: payerKp.address,
      mint: rewardMint.address,
    }),
    getMintToInstruction({
      mint: rewardMint.address,
      token: payerAta,
      mintAuthority: deployer,
      amount: payAmount,
    }),
  ]);

  // 1. pay_traffic → split + burn.
  const supplyBefore = await supplyOf(conn, rewardMint.address);
  const vaultBefore = await tokenAmount(conn, rewardVaultPda);
  const treasuryBefore = await tokenAmount(conn, treasury);
  await send(conn, deployer, [
    await rewardsSettlement.getPayTrafficInstructionAsync({
      payer: payerSigner,
      rewardMint: rewardMint.address,
      payerTokenAccount: payerAta,
      rewardVault: rewardVaultPda,
      treasury,
      amount: payAmount,
    }),
  ]);
  const split = math.splitPayment(payAmount);
  const vaultDelta = (await tokenAmount(conn, rewardVaultPda)) - vaultBefore;
  const treasuryDelta = (await tokenAmount(conn, treasury)) - treasuryBefore;
  const burned = supplyBefore - (await supplyOf(conn, rewardMint.address));
  console.log(
    `[m4] pay_traffic ${payAmount}: vault +${vaultDelta} (exp ${split.nodes}), treasury +${treasuryDelta} (exp ${split.treasury}), burned ${burned} (exp ${split.burn})`,
  );
  if (vaultDelta !== split.nodes) throw new Error('vault split mismatch');
  if (burned !== split.burn) throw new Error('burn mismatch');

  // 2. fund_vault.
  await send(conn, deployer, [
    await rewardsSettlement.getFundVaultInstructionAsync({
      funder: deployer,
      rewardMint: rewardMint.address,
      funderTokenAccount: deployerRewardAta,
      rewardVault: rewardVaultPda,
      amount: 1_000_000_000n,
    }),
  ]);

  // 3. Build + post epoch E1.
  const nodeInfos = (await fetchNodeInfos(conn.rpc)).filter(
    (n) => n.operator === deployer.address && n.nodeId === NODE_ID,
  );
  if (nodeInfos.length === 0) throw new Error('live node 1 not found in NodeState');
  const clientKp = makeRawKeypair();

  const e1 = baseEpoch + 1n;
  const r1 = makeReceipt(clientKp.address, deployer.address, clientKp.seed, operatorSeed, {
    nodeId: NODE_ID,
    bytes: 1_000_000_000n,
    epoch: e1,
    nonce: 1n,
  });
  const build1 = buildEpoch(e1, [r1], nodeInfos, { minStakeToEarn: 1n });
  if (build1.numNodes !== 1) throw new Error(`epoch empty: ${JSON.stringify(build1.skipped)}`);
  const entry1 = build1.entries[0];
  console.log(`[m4] epoch ${e1}: reward ${entry1.amount}, root ${build1.root}`);
  await send(conn, deployer, [
    await rewardsSettlement.getPostEpochInstructionAsync({
      poster: deployer,
      rewardVault: rewardVaultPda,
      epoch: e1,
      merkleRoot: math.fromHex(build1.root),
      totalReward: build1.totalReward,
      numNodes: build1.numNodes,
    }),
  ]);

  // 4. Claim after the dispute window.
  await sleep(3_000);
  const before = await tokenAmount(conn, deployerRewardAta);
  await send(conn, deployer, [
    await rewardsSettlement.getClaimInstructionAsync({
      claimant: deployer,
      operator: deployer.address,
      rewardMint: rewardMint.address,
      rewardVault: rewardVaultPda,
      operatorTokenAccount: deployerRewardAta,
      epoch: e1,
      nodeId: NODE_ID,
      amount: entry1.amount,
      proof: entry1.proof.map(math.fromHex),
    }),
  ]);
  const credited = (await tokenAmount(conn, deployerRewardAta)) - before;
  console.log(`[m4] claim credited ${credited} (exp ${entry1.amount})`);
  if (credited !== entry1.amount) throw new Error('claim payout mismatch');

  // 5. Dispute epoch E2's leaf.
  const e2 = baseEpoch + 2n;
  const r2 = makeReceipt(clientKp.address, deployer.address, clientKp.seed, operatorSeed, {
    nodeId: NODE_ID,
    bytes: 1_000_000_000n,
    epoch: e2,
    nonce: 2n,
  });
  const build2 = buildEpoch(e2, [r2], nodeInfos, { minStakeToEarn: 1n });
  const entry2 = build2.entries[0];
  await send(conn, deployer, [
    await rewardsSettlement.getPostEpochInstructionAsync({
      poster: deployer,
      rewardVault: rewardVaultPda,
      epoch: e2,
      merkleRoot: math.fromHex(build2.root),
      totalReward: build2.totalReward,
      numNodes: build2.numNodes,
    }),
  ]);
  await send(conn, deployer, [
    staking.getSetSlashAuthorityInstruction({
      authority: deployer,
      config: stakingConfig,
      newSlashAuthority: settlementAuth,
    }),
    reputation.getSetOracleInstruction({
      authority: deployer,
      config: repConfig,
      newOracle: settlementAuth,
    }),
  ]);
  console.log('[m4] slash_authority + oracle re-pointed at settlement PDA');

  const nsBefore = await fetchNodeStateTolerant(conn.rpc, node);
  const slashAmount = 10_000n;
  await send(conn, deployer, [
    await rewardsSettlement.getDisputeInstructionAsync({
      disputeAuthority: deployer,
      operator: deployer.address,
      stakingConfig,
      stakingPosition: position,
      stakingVault: pos0.data.vault,
      stakingTreasury: stakeTreasury,
      stakeMint,
      stakingProgramAuthority: stakingAuth,
      reputationConfig: repConfig,
      reputationState: repState,
      reputationProgramAuthority: repAuth,
      registry,
      node,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
      epoch: e2,
      nodeId: NODE_ID,
      amount: entry2.amount,
      severityBps: 5_000,
      slashAmount,
    }),
  ]);
  const posAfter = await staking.fetchStakePosition(conn.rpc, position);
  const nsAfter = await fetchNodeStateTolerant(conn.rpc, node);
  console.log(
    `[m4] dispute: position ${pos0.data.amount} → ${posAfter.data.amount}; NodeState stake ${nsBefore.data.stakeAmount} → ${nsAfter.data.stakeAmount}, reputation ${nsBefore.data.reputation} → ${nsAfter.data.reputation} bps`,
  );
  if (pos0.data.amount - posAfter.data.amount !== slashAmount) throw new Error('slash mismatch');
  if (nsAfter.data.stakeAmount !== posAfter.data.amount) throw new Error('stake mirror mismatch');
  if (nsAfter.data.reputation >= nsBefore.data.reputation)
    throw new Error('reputation not penalized');

  let blocked = false;
  try {
    await send(conn, deployer, [
      await rewardsSettlement.getClaimInstructionAsync({
        claimant: deployer,
        operator: deployer.address,
        rewardMint: rewardMint.address,
        rewardVault: rewardVaultPda,
        operatorTokenAccount: deployerRewardAta,
        epoch: e2,
        nodeId: NODE_ID,
        amount: entry2.amount,
        proof: entry2.proof.map(math.fromHex),
      }),
    ]);
  } catch {
    blocked = true;
  }
  if (!blocked) throw new Error('disputed leaf was claimable!');
  console.log('[m4] ✅ pay→post→claim→dispute verified on devnet; disputed leaf unclaimable');
}

main().catch((e) => {
  console.error('[m4] failed:', e);
  process.exit(1);
});
