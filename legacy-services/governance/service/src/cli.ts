// Governance CLI: read the live DAO config and a proposal's tally. A thin
// inspector over the on-chain accounts (proposal authoring/execution is driven
// by the m5 smoke and, in production, a front-end using @weft/sdk builders).

import { createSolanaRpc } from '@solana/kit';
import { governance } from '@weft/sdk';

async function main(): Promise<void> {
  const rpcUrl = process.env.WEFT_RPC_URL ?? 'https://api.devnet.solana.com';
  const rpc = createSolanaRpc(rpcUrl);

  const [configPda] = await governance.findGovernanceConfigPda();
  const cfg = await governance.fetchMaybeGovernanceConfig(rpc, configPda);
  if (!cfg.exists) {
    console.log('[gov] governance not initialized on this cluster');
    return;
  }
  console.log(`[gov] proposals: ${cfg.data.proposalCount}`);
  console.log(
    `[gov] quorum: ${cfg.data.defaultQuorum}, threshold: ${cfg.data.defaultApprovalThresholdBps} bps`,
  );

  const [pcPda] = await governance.findProtocolConfigPda();
  const pc = await governance.fetchMaybeProtocolConfig(rpc, pcPda);
  if (pc.exists) {
    console.log(
      `[gov] split nodes/burn/treasury = ${pc.data.splitNodesBps}/${pc.data.splitBurnBps}/${pc.data.splitTreasuryBps} bps`,
    );
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
