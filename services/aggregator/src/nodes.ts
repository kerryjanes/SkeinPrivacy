// Reward inputs read straight from the authoritative `NodeState` PDAs via
// getProgramAccounts (the same source the M2 indexer serves). Devnet Helius DAS
// does not index Bubblegum V2, so `NodeState` — not DAS — is the directory of
// record for reputation and stake.

import {
  createSolanaRpc,
  getBase58Decoder,
  type Address,
  type Base58EncodedBytes,
} from '@solana/kit';
import { weft } from '@weft/sdk';

import type { NodeInfo } from './rewards';

export type Rpc = ReturnType<typeof createSolanaRpc>;

const toHex = (u: ArrayLike<number>): string =>
  Array.from(u, (b) => b.toString(16).padStart(2, '0')).join('');

export interface FetchNodeInfosOptions {
  /** Legacy option kept for CLI compatibility; ignored by the single-core program. */
  merkleTree?: Address | null;
}

/** Decode a `NodeState` tolerating the pre-M8 (shorter, no trailing `sequence`) layout
 *  that devnet's node-registry still uses — zero-padding to the decoder's expected size. */
export async function fetchNodeStateTolerant(
  client: Rpc,
  addr: Address,
): Promise<{ data: ReturnType<ReturnType<typeof weft.getNodeStateDecoder>['decode']> }> {
  const info = await client.getAccountInfo(addr, { encoding: 'base64' }).send();
  if (!info.value) throw new Error(`NodeState not found: ${addr}`);
  let bytes = Buffer.from(info.value.data[0], 'base64');
  const decoder = weft.getNodeStateDecoder();
  if (bytes.length < decoder.fixedSize) {
    const padded = Buffer.alloc(decoder.fixedSize);
    bytes.copy(padded);
    bytes = padded;
  }
  return { data: decoder.decode(bytes) };
}

export function rewardMerkleTreeFromEnv(): Address | null {
  return process.env.WEFT_MERKLE_TREE ? (process.env.WEFT_MERKLE_TREE as Address) : null;
}

export async function fetchNodeInfos(
  client: Rpc,
  opts: FetchNodeInfosOptions = {},
): Promise<NodeInfo[]> {
  const disc = getBase58Decoder().decode(weft.NODE_STATE_DISCRIMINATOR);
  void opts;
  const accounts = await client
    .getProgramAccounts(weft.WEFT_PROGRAM_ADDRESS, {
      encoding: 'base64',
      filters: [{ memcmp: { offset: 0n, bytes: disc as Base58EncodedBytes, encoding: 'base58' } }],
    })
    .send();
  const decoder = weft.getNodeStateDecoder();
  return accounts.map(({ account }) => {
    let bytes = Buffer.from((account.data as readonly [string, string])[0], 'base64');
    // Devnet's node-registry predates M8, so NodeState lacks the trailing `sequence`
    // u64. The program documents pre-M8 accounts as "read 0 here", so zero-pad to the
    // decoder's expected size (matching registry-provision's fetchNodeStateTolerant).
    if (bytes.length < decoder.fixedSize) {
      const padded = Buffer.alloc(decoder.fixedSize);
      bytes.copy(padded);
      bytes = padded;
    }
    const d = decoder.decode(bytes);
    return {
      operator: d.operator,
      nodeId: d.nodeId,
      endpointHash: toHex(d.endpointHash),
      reputationBps: d.reputation,
      geo: d.geo,
      stake: d.stakeAmount,
    };
  });
}
