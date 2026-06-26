// M4 devnet smoke: exercise the full settlement economic loop against the LIVE
// programs and the real node 1 (operated by the deployer, staked + scored in M3):
//   1. pay_traffic  → observe the 70/20/10 split + burn (supply drop)
//   2. escrow path  → deposit, settle from prepaid escrow, withdraw unused
//   3. fund_vault   → top up the reward pool
//   4. build an epoch from a synthetic dual-signed receipt for node 1, post it
//   5. claim with the served proof → operator ATA credited
//   6. re-point slash_authority/oracle → settlement PDA, dispute a second epoch's
//      leaf → live StakePosition slashed + reputation penalized (mirrored into
//      NodeState), and the disputed leaf becomes unclaimable.
//
// Reward token is the fresh devnet genesis $WEFT mint from staking config, so
// settlement exercises the same token as genesis, staking, cabinet, and rewards.
// Run: WEFT_KEYPAIR=… WEFT_RPC_URL=<rpc> pnpm --filter @weft/aggregator smoke

import { randomBytes } from 'node:crypto';
import { ed25519 } from '@noble/curves/ed25519';
import {
  createKeyPairSignerFromBytes,
  getAddressDecoder,
  getProgramDerivedAddress,
  lamports,
  type Address,
} from '@solana/kit';
import { getTransferSolInstruction } from '@solana-program/system';
import {
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstructionAsync,
  getTransferCheckedInstruction,
  TOKEN_PROGRAM_ADDRESS,
} from '@solana-program/token';
import { math, nodeRegistry, reputation, rewardsSettlement, staking } from '@weft/sdk';

import { connect, loadEd25519Seed, loadSigner, send, type Connection } from './kit';
import { buildEpoch } from './rewards';
import { signReceiptCore, type TrafficReceipt } from './receipts';
import { epochRange } from './epoch';
import { fetchNodeInfos, fetchNodeStateTolerant } from './nodes';

const NODE_ID = 1n;
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
  const rewardMint = cfg.data.mint;
  const stakeTreasury = cfg.data.treasury;
  const pos0 = await staking.fetchStakePosition(conn.rpc, position);
  console.log(`[m4] reward/stake mint ${rewardMint}; position amount ${pos0.data.amount}`);

  const deployerRewardAta = await ata(deployer.address, rewardMint);
  const distMaybe = await rewardsSettlement.fetchMaybeDistributor(conn.rpc, distributorPda);
  await send(conn, deployer, [
    await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: deployer,
      owner: deployer.address,
      mint: rewardMint,
    }),
  ]);

  if (!distMaybe.exists) {
    await send(conn, deployer, [
      await rewardsSettlement.getInitializeDistributorInstructionAsync({
        authority: deployer,
        rewardMint,
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
  if (dist.data.rewardMint !== rewardMint)
    throw new Error(`distributor mint ${dist.data.rewardMint} != genesis mint ${rewardMint}`);
  const treasury = dist.data.treasury;
  const baseEpoch = dist.data.currentEpoch;

  // pay_traffic needs a payer whose token account differs from the treasury
  // (Anchor forbids the same account appearing twice as mutable). Fund a fresh
  // payer with SOL + reward tokens for the demo.
  const payerKp = makeRawKeypair();
  const payerSigner = await createKeyPairSignerFromBytes(payerKp.secret64);
  const payerAta = await ata(payerKp.address, rewardMint);
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
      mint: rewardMint,
    }),
    getTransferCheckedInstruction({
      source: deployerRewardAta,
      mint: rewardMint,
      destination: payerAta,
      authority: deployer,
      amount: payAmount,
      decimals: 9,
    }),
  ]);

  // 1. pay_traffic → split + burn.
  const supplyBefore = await supplyOf(conn, rewardMint);
  const vaultBefore = await tokenAmount(conn, rewardVaultPda);
  const treasuryBefore = await tokenAmount(conn, treasury);
  await send(conn, deployer, [
    await rewardsSettlement.getPayTrafficInstructionAsync({
      payer: payerSigner,
      rewardMint,
      payerTokenAccount: payerAta,
      rewardVault: rewardVaultPda,
      treasury,
      amount: payAmount,
    }),
  ]);
  const split = math.splitPayment(payAmount);
  const vaultDelta = (await tokenAmount(conn, rewardVaultPda)) - vaultBefore;
  const treasuryDelta = (await tokenAmount(conn, treasury)) - treasuryBefore;
  const burned = supplyBefore - (await supplyOf(conn, rewardMint));
  console.log(
    `[m4] pay_traffic ${payAmount}: vault +${vaultDelta} (exp ${split.nodes}), treasury +${treasuryDelta} (exp ${split.treasury}), burned ${burned} (exp ${split.burn})`,
  );
  if (vaultDelta !== split.nodes) throw new Error('vault split mismatch');
  if (burned !== split.burn) throw new Error('burn mismatch');

  // 2. Escrow-first path: deposit → settle from escrow → withdraw unused.
  const escrowKp = makeRawKeypair();
  const escrowSigner = await createKeyPairSignerFromBytes(escrowKp.secret64);
  const escrowAta = await ata(escrowKp.address, rewardMint);
  const [escrowPda] = await rewardsSettlement.findEscrowPda({ owner: escrowKp.address });
  const [escrowVault] = await rewardsSettlement.findEscrowVaultPda({ owner: escrowKp.address });
  const escrowDeposit = 800_000_000n;
  const escrowSettle = 500_000_000n;
  const escrowWithdraw = 300_000_000n;
  await send(conn, deployer, [
    getTransferSolInstruction({
      source: deployer,
      destination: escrowKp.address,
      amount: lamports(20_000_000n),
    }),
  ]);
  await send(conn, deployer, [
    await getCreateAssociatedTokenIdempotentInstructionAsync({
      payer: deployer,
      owner: escrowKp.address,
      mint: rewardMint,
    }),
    getTransferCheckedInstruction({
      source: deployerRewardAta,
      mint: rewardMint,
      destination: escrowAta,
      authority: deployer,
      amount: escrowDeposit,
      decimals: 9,
    }),
  ]);
  await send(conn, deployer, [
    await rewardsSettlement.getDepositEscrowInstructionAsync({
      owner: escrowSigner,
      rewardMint,
      ownerTokenAccount: escrowAta,
      amount: escrowDeposit,
    }),
  ]);
  let escrowState = await rewardsSettlement.fetchPaymentEscrow(conn.rpc, escrowPda);
  if (escrowState.data.balance !== escrowDeposit) throw new Error('escrow deposit mismatch');

  const escrowVaultBefore = await tokenAmount(conn, rewardVaultPda);
  const escrowTreasuryBefore = await tokenAmount(conn, treasury);
  const escrowSupplyBefore = await supplyOf(conn, rewardMint);
  await send(conn, deployer, [
    await rewardsSettlement.getPayTrafficFromEscrowInstructionAsync({
      owner: escrowSigner,
      escrowVault,
      rewardMint,
      rewardVault: rewardVaultPda,
      treasury,
      amount: escrowSettle,
    }),
  ]);
  const escrowSplit = math.splitPayment(escrowSettle);
  const escrowVaultDelta = (await tokenAmount(conn, rewardVaultPda)) - escrowVaultBefore;
  const escrowTreasuryDelta = (await tokenAmount(conn, treasury)) - escrowTreasuryBefore;
  const escrowBurned = escrowSupplyBefore - (await supplyOf(conn, rewardMint));
  escrowState = await rewardsSettlement.fetchPaymentEscrow(conn.rpc, escrowPda);
  if (escrowState.data.balance !== escrowWithdraw) throw new Error('escrow settle balance mismatch');
  if (escrowVaultDelta !== escrowSplit.nodes) throw new Error('escrow vault split mismatch');
  if (escrowTreasuryDelta !== escrowSplit.treasury) throw new Error('escrow treasury split mismatch');
  if (escrowBurned !== escrowSplit.burn) throw new Error('escrow burn mismatch');

  await send(conn, deployer, [
    await rewardsSettlement.getWithdrawEscrowInstructionAsync({
      owner: escrowSigner,
      escrowVault,
      rewardMint,
      ownerTokenAccount: escrowAta,
      amount: escrowWithdraw,
    }),
  ]);
  escrowState = await rewardsSettlement.fetchPaymentEscrow(conn.rpc, escrowPda);
  if (escrowState.data.balance !== 0n) throw new Error('escrow withdraw mismatch');
  console.log(
    `[m4] escrow deposit ${escrowDeposit}, settle ${escrowSettle}: vault +${escrowVaultDelta}, treasury +${escrowTreasuryDelta}, burned ${escrowBurned}; withdraw ${escrowWithdraw}`,
  );

  // 3. fund_vault.
  await send(conn, deployer, [
    await rewardsSettlement.getFundVaultInstructionAsync({
      funder: deployer,
      rewardMint,
      funderTokenAccount: deployerRewardAta,
      rewardVault: rewardVaultPda,
      amount: 1_000_000_000n,
    }),
  ]);

  // 4. Build + post epoch E1.
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

  // 5. Claim after the dispute window.
  await sleep(3_000);
  const before = await tokenAmount(conn, deployerRewardAta);
  await send(conn, deployer, [
    await rewardsSettlement.getClaimInstructionAsync({
      claimant: deployer,
      operator: deployer.address,
      rewardMint,
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

  // 6. Dispute epoch E2's leaf.
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
  const slashAmount = pos0.data.amount / 2n;
  if (slashAmount <= 0n) throw new Error('operator stake is too small to run dispute smoke');
  const disputeIx = await rewardsSettlement.getDisputeInstructionAsync({
    disputeAuthority: deployer,
    operator: deployer.address,
    stakingConfig,
    stakingPosition: position,
    stakingVault: pos0.data.vault,
    stakingTreasury: stakeTreasury,
    stakeMint: rewardMint,
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
  });
  let disputeError: unknown;
  try {
    await send(conn, deployer, [disputeIx]);
  } catch (e) {
    disputeError = e;
  }

  const [claimStatusPda] = await rewardsSettlement.findClaimStatusPda({
    epoch: e2,
    operator: deployer.address,
    nodeId: NODE_ID,
  });
  const claimStatus = await rewardsSettlement.fetchMaybeClaimStatus(conn.rpc, claimStatusPda);
  if (!claimStatus.exists || !claimStatus.data.disputed) {
    if (disputeError) throw disputeError;
    throw new Error('dispute did not create a disputed ClaimStatus');
  }

  const posAfter = await staking.fetchStakePosition(conn.rpc, position);
  const nsAfter = await fetchNodeStateTolerant(conn.rpc, node);
  console.log(
    `[m4] dispute: position ${pos0.data.amount} → ${posAfter.data.amount}; NodeState stake ${nsBefore.data.stakeAmount} → ${nsAfter.data.stakeAmount}, reputation ${nsBefore.data.reputation} → ${nsAfter.data.reputation} bps`,
  );
  if (pos0.data.amount - posAfter.data.amount !== slashAmount) throw new Error('slash mismatch');
  if (nsAfter.data.stakeAmount !== posAfter.data.amount) throw new Error('stake mirror mismatch');
  if (nsAfter.data.reputation >= nsBefore.data.reputation)
    throw new Error('reputation not penalized');
  console.log(
    `[m4] disputed claim status ${claimStatusPda}: amount ${claimStatus.data.amount}`,
  );
  console.log('[m4] ✅ pay→post→claim→dispute verified on devnet; disputed leaf blocked');
}

main().catch((e) => {
  console.error('[m4] failed:', e);
  process.exit(1);
});
