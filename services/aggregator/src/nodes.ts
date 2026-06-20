// Reward inputs read straight from the authoritative `NodeState` PDAs via
// getProgramAccounts (the same source the M2 indexer serves). Devnet Helius DAS
// does not index Bubblegum V2, so `NodeState` — not DAS — is the directory of
// record for reputation and stake.

import { createSolanaRpc, getBase58Decoder, type Base58EncodedBytes } from '@solana/kit';
import { nodeRegistry } from '@weft/sdk';

import type { NodeInfo } from './rewards';

export type Rpc = ReturnType<typeof createSolanaRpc>;

export async function fetchNodeInfos(client: Rpc): Promise<NodeInfo[]> {
  const disc = getBase58Decoder().decode(nodeRegistry.NODE_STATE_DISCRIMINATOR);
  const accounts = await client
    .getProgramAccounts(nodeRegistry.NODE_REGISTRY_PROGRAM_ADDRESS, {
      encoding: 'base64',
      filters: [{ memcmp: { offset: 0n, bytes: disc as Base58EncodedBytes, encoding: 'base58' } }],
    })
    .send();
  const decoder = nodeRegistry.getNodeStateDecoder();
  return accounts.map(({ account }) => {
    const bytes = Buffer.from((account.data as readonly [string, string])[0], 'base64');
    const d = decoder.decode(bytes);
    return {
      operator: d.operator,
      nodeId: d.nodeId,
      reputationBps: d.reputation,
      geo: d.geo,
      stake: d.stakeAmount,
    };
  });
}
