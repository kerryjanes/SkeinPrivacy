// Node directory built from the authoritative NodeState PDAs via
// getProgramAccounts (works without DAS — required while Helius devnet lacks
// Bubblegum V2 indexing). Each NodeState now carries the real Bubblegum cNFT
// asset id + tree, so the directory reports them directly; DAS can further
// enrich owner/metadata where Bubblegum V2 indexing is available.

import { createSolanaRpc, getBase58Decoder, type Base58EncodedBytes } from '@solana/kit';
import { weft } from '@weft/sdk';

export const NODE_REGISTRY_PROGRAM = weft.WEFT_PROGRAM_ADDRESS;
export const GEO_BITS = 30;
export const GEO_MAX = ((1 << GEO_BITS) - 1) >>> 0;

/** Mirrors `weft_primitives::geo_region_prefix`. */
export function geoRegionPrefix(geo: number, chars: number): number {
  const keep = Math.min(chars, 6) * 5;
  const g = geo & GEO_MAX;
  return keep >= GEO_BITS ? g : g >>> (GEO_BITS - keep);
}

export interface NodeRecord {
  address: string;
  operator: string;
  nodeId: string;
  assetId: string;
  merkleTree: string;
  geo: number;
  capabilities: number;
  availability: number;
  status: number;
  reputation: number;
  stake: string;
  updatedAt: number;
}

export type Rpc = ReturnType<typeof createSolanaRpc>;
export const rpc = (url: string): Rpc => createSolanaRpc(url);

export async function fetchNodes(client: Rpc): Promise<NodeRecord[]> {
  const disc = getBase58Decoder().decode(weft.NODE_STATE_DISCRIMINATOR);
  const accounts = await client
    .getProgramAccounts(NODE_REGISTRY_PROGRAM, {
      encoding: 'base64',
      filters: [{ memcmp: { offset: 0n, bytes: disc as Base58EncodedBytes, encoding: 'base58' } }],
    })
    .send();
  const decoder = weft.getNodeStateDecoder();
  return accounts.map(({ pubkey, account }) => {
    let bytes = Buffer.from((account.data as readonly [string, string])[0], 'base64');
    if (bytes.length < decoder.fixedSize) {
      const padded = Buffer.alloc(decoder.fixedSize);
      bytes.copy(padded);
      bytes = padded;
    }
    const d = decoder.decode(bytes);
    return {
      address: pubkey,
      operator: d.operator,
      nodeId: d.nodeId.toString(),
      assetId: d.assetId,
      merkleTree: d.merkleTree,
      geo: d.geo,
      capabilities: d.capabilities,
      availability: d.availability,
      status: d.status,
      reputation: d.reputation,
      stake: d.stakeAmount.toString(),
      updatedAt: Number(d.updatedAt),
    };
  });
}

export interface NodeQuery {
  geoPrefix?: { region: number; chars: number };
  minAvailability?: number;
  capability?: number;
  minReputation?: number;
  activeOnly?: boolean;
}

export function filterNodes(nodes: NodeRecord[], q: NodeQuery): NodeRecord[] {
  return nodes.filter((n) => {
    if ((q.activeOnly ?? true) && n.status !== 0) return false;
    if (q.minAvailability != null && n.availability < q.minAvailability) return false;
    if (q.minReputation != null && n.reputation < q.minReputation) return false;
    if (q.capability != null && (n.capabilities & q.capability) === 0) return false;
    if (q.geoPrefix && geoRegionPrefix(n.geo, q.geoPrefix.chars) !== q.geoPrefix.region)
      return false;
    return true;
  });
}
