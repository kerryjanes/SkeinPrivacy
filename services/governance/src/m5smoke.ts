// M5 devnet smoke: prove the live DAO end-to-end. Bootstrap governance +
// ProtocolConfig (config authority = the DAO PDA), stake a freshly-locked
// position, then run a full proposal that EDITS the governed ProtocolConfig:
//   create → add_transaction(update_protocol_config) → activate → vote (staked) →
//   finalize → timelock → execute → read the new value on-chain.
//
// The proposal's voting power is real staked $WEFT (a locked StakePosition); the
// executed instruction is signed by the governance authority PDA via invoke_signed.
// Run: WEFT_KEYPAIR=… WEFT_RPC_URL=<helius> pnpm --filter @weft/governance smoke

import {
  createNoopSigner,
  getAddressEncoder,
  getProgramDerivedAddress,
  type Address,
} from '@solana/kit';
import { findAssociatedTokenPda, TOKEN_PROGRAM_ADDRESS } from '@solana-program/token';
import { governance, staking } from '@weft/sdk';

import { connect, loadSigner, send, type Connection } from './kit';
import { encodeInstruction, executeRemainingAccounts } from './proposal';

const NODE_ID = 99n; // a governance-only stake position (no registered node)
const STAKE_AMOUNT = 15_000n;
const LOCK_SECONDS = 3_600n;
const addrEnc = getAddressEncoder();
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function u64le(v: bigint): Uint8Array {
  const out = new Uint8Array(8);
  let x = v;
  for (let i = 0; i < 8; i++) {
    out[i] = Number(x & 0xffn);
    x >>= 8n;
  }
  return out;
}
function u16le(v: number): Uint8Array {
  return new Uint8Array([v & 0xff, (v >> 8) & 0xff]);
}

async function proposalPda(id: bigint): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: governance.GOVERNANCE_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('proposal'), u64le(id)],
  });
  return pda;
}
async function proposalTxPda(proposal: Address, index: number): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: governance.GOVERNANCE_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('proposal_tx'), addrEnc.encode(proposal), u16le(index)],
  });
  return pda;
}

async function registryPda(): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: (await import('@weft/sdk')).nodeRegistry.NODE_REGISTRY_PROGRAM_ADDRESS,
    seeds: [new TextEncoder().encode('registry')],
  });
  return pda;
}

async function main(): Promise<void> {
  const rpcUrl = process.env.WEFT_RPC_URL ?? 'https://api.devnet.solana.com';
  const wsUrl = process.env.WEFT_RPC_WS ?? rpcUrl.replace(/^http/, 'ws').replace('8899', '8900');
  const keypairPath = process.env.WEFT_KEYPAIR ?? `${process.env.HOME}/.config/solana/id.json`;
  const conn: Connection = connect(rpcUrl, wsUrl);
  const deployer = await loadSigner(keypairPath);
  console.log(`[m5] deployer ${deployer.address}`);

  const [govConfigPda] = await governance.findGovernanceConfigPda();
  const [govAuthority] = await governance.findGovernanceAuthorityPda();
  const [protocolConfigPda] = await governance.findProtocolConfigPda();
  const cfg = await staking.fetchStakingConfig(conn.rpc, (await staking.findConfigPda())[0]);
  const stakeMint = cfg.data.mint;

  // 1. Bootstrap governance (idempotent).
  const gc = await governance.fetchMaybeGovernanceConfig(conn.rpc, govConfigPda);
  if (!gc.exists) {
    await send(conn, deployer, [
      await governance.getInitializeGovernanceInstructionAsync({
        authority: deployer,
        govMint: stakeMint,
        defaultQuorum: 10_000n,
        defaultApprovalThresholdBps: 6_000,
        votingPeriodSeconds: 30n,
        executionDelaySeconds: 5n,
        minProposalStake: 10_000n,
      }),
    ]);
    console.log('[m5] governance initialized');
  }
  const pcExists = await governance.fetchMaybeProtocolConfig(conn.rpc, protocolConfigPda);
  if (!pcExists.exists) {
    await send(conn, deployer, [
      await governance.getInitializeProtocolConfigInstructionAsync({
        authority: deployer,
        configAuthority: govAuthority, // only the DAO PDA can update it
        disputeWindowSeconds: 300n,
        clawbackWindowSeconds: 600n,
      }),
    ]);
    console.log('[m5] protocol config initialized (authority = DAO PDA)');
  }

  // 2. Stake a freshly-locked governance position (lock pushes locked_until far
  //    enough that it can vote through the proposal window).
  const opAta = (
    await findAssociatedTokenPda({
      owner: deployer.address,
      mint: stakeMint,
      tokenProgram: TOKEN_PROGRAM_ADDRESS,
    })
  )[0];
  await send(conn, deployer, [
    await staking.getStakeInstructionAsync({
      operator: deployer,
      operatorTokenAccount: opAta,
      mint: stakeMint,
      registry: await registryPda(),
      nodeId: NODE_ID,
      amount: STAKE_AMOUNT,
      lockDuration: LOCK_SECONDS,
    }),
  ]);
  const [position] = await staking.findPositionPda({ operator: deployer.address, nodeId: NODE_ID });
  const pos = await staking.fetchStakePosition(conn.rpc, position);
  console.log(
    `[m5] staked position ${position}: amount ${pos.data.amount}, locked_until ${pos.data.lockedUntil}`,
  );

  // 3. Create the proposal; its tx flips the governed dispute window.
  const cfgNow = await governance.fetchGovernanceConfig(conn.rpc, govConfigPda);
  const proposalId = cfgNow.data.proposalCount;
  const proposal = await proposalPda(proposalId);
  await send(conn, deployer, [
    await governance.getCreateProposalInstructionAsync({
      proposer: deployer,
      position,
      proposal,
      nodeId: NODE_ID,
      name: 'flip dispute window',
    }),
  ]);
  console.log(`[m5] created proposal ${proposalId}`);

  const pcBefore = await governance.fetchProtocolConfig(conn.rpc, protocolConfigPda);
  const newWindow = pcBefore.data.disputeWindowSeconds === 300n ? 301n : 300n;
  const innerIx = await governance.getUpdateProtocolConfigInstructionAsync({
    authority: createNoopSigner(govAuthority),
    protocolConfig: protocolConfigPda,
    splitNodesBps: pcBefore.data.splitNodesBps,
    splitBurnBps: pcBefore.data.splitBurnBps,
    splitTreasuryBps: pcBefore.data.splitTreasuryBps,
    disputeWindowSeconds: newWindow,
    clawbackWindowSeconds: pcBefore.data.clawbackWindowSeconds,
    baseRatePerGb: pcBefore.data.baseRatePerGb,
    geoBonusMaxBps: pcBefore.data.geoBonusMaxBps,
    reputationMinBps: pcBefore.data.reputationMinBps,
    reputationMaxBps: pcBefore.data.reputationMaxBps,
    stakingBonusBps: pcBefore.data.stakingBonusBps,
    stakingBonusThreshold: pcBefore.data.stakingBonusThreshold,
    bootstrapNodeLimit: pcBefore.data.bootstrapNodeLimit,
    bootstrapBonusBps: pcBefore.data.bootstrapBonusBps,
    bootstrapEndTs: pcBefore.data.bootstrapEndTs,
  });
  const queued = encodeInstruction(innerIx);
  await send(conn, deployer, [
    await governance.getAddTransactionInstructionAsync({
      proposer: deployer,
      proposal,
      proposalTransaction: await proposalTxPda(proposal, 0),
      index: 0,
      programId: queued.programId,
      accounts: queued.accounts,
      data: queued.data,
    }),
  ]);
  await send(conn, deployer, [
    await governance.getActivateProposalInstructionAsync({ proposer: deployer, proposal }),
  ]);
  console.log('[m5] proposal activated; voting open');

  // 4. Vote the staked weight.
  await send(conn, deployer, [
    await governance.getCastVoteInstructionAsync({
      voter: deployer,
      position,
      proposal,
      nodeId: NODE_ID,
      vote: governance.VoteKind.Yes,
    }),
  ]);
  console.log('[m5] voted Yes with staked weight');

  // 5. Finalize after the voting window.
  console.log('[m5] waiting out the 30s voting window…');
  await sleep(34_000);
  await send(conn, deployer, [
    await governance.getFinalizeProposalInstructionAsync({ finalizer: deployer, proposal }),
  ]);
  const finalized = await governance.fetchProposal(conn.rpc, proposal);
  console.log(`[m5] proposal state: ${finalized.data.state}`);

  // 6. Execute after the timelock.
  console.log('[m5] waiting out the 5s timelock…');
  await sleep(7_000);
  const baseExec = await governance.getExecuteTransactionInstructionAsync({
    executor: deployer,
    proposal,
    proposalTransaction: await proposalTxPda(proposal, 0),
  });
  const execIx = {
    ...baseExec,
    accounts: [...baseExec.accounts, ...executeRemainingAccounts(queued, govAuthority)],
  };
  await send(conn, deployer, [execIx]);

  const pcAfter = await governance.fetchProtocolConfig(conn.rpc, protocolConfigPda);
  console.log(
    `[m5] dispute_window ${pcBefore.data.disputeWindowSeconds} → ${pcAfter.data.disputeWindowSeconds} (expected ${newWindow})`,
  );
  if (pcAfter.data.disputeWindowSeconds !== newWindow)
    throw new Error('ProtocolConfig was not updated by the proposal');
  console.log('[m5] ✅ DAO proposal passed and edited the governed ProtocolConfig on devnet');
}

main().catch((e) => {
  console.error('[m5] failed:', e);
  process.exit(1);
});
