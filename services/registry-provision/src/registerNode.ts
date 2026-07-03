import type { TransactionSigner } from '@solana/kit';
import { weft } from '@weft/sdk';

import { send, type Connection } from './kit';
import {
  BUBBLEGUM_PROGRAM,
  MPL_CORE_PROGRAM,
  mintAccountsFromChain,
  NODE_METADATA_URI,
} from './nft';

export interface NodeParams {
  nodeId: bigint;
  geo: number;
  capabilities: number;
  availability: number;
  /** On-chain commitment over the node's stable identity. Defaults to zeros if omitted. */
  endpointHash?: Uint8Array;
  /** Off-chain metadata URI for the node cNFT. Defaults to the canonical node.json. */
  metadataUri?: string;
}

/**
 * Register a node through the single Weft core program. This mints the node's
 * Bubblegum V2 cNFT (identity token, owned by the operator) and creates the
 * NodeState PDA. The collection + active tree must already be provisioned (see
 * `provision.ts`); their addresses come from the cluster manifest.
 */
export async function registerNode(
  conn: Connection,
  operator: TransactionSigner,
  params: NodeParams,
  _cluster = 'devnet',
): Promise<string> {
  // Read the collection + active tree from the authoritative on-chain Registry
  // (no bundled manifest needed — a standalone node-agent works anywhere the
  // chain is provisioned).
  const mint = await mintAccountsFromChain(conn.rpc, params.metadataUri ?? NODE_METADATA_URI);

  const ix = await weft.getRegisterNodeInstructionAsync({
    operator,
    nodeId: params.nodeId,
    geo: params.geo,
    capabilities: params.capabilities,
    endpointHash: params.endpointHash ?? new Uint8Array(32),
    availability: params.availability,
    metadataUri: mint.metadataUri,
    treeShard: mint.treeShard,
    treeConfig: mint.treeConfig,
    merkleTree: mint.merkleTree,
    coreCollection: mint.coreCollection,
    mplCoreCpiSigner: mint.mplCoreCpiSigner,
    logWrapper: mint.logWrapper,
    compressionProgram: mint.compressionProgram,
    mplCoreProgram: MPL_CORE_PROGRAM,
    bubblegumProgram: BUBBLEGUM_PROGRAM,
  });
  return send(conn, operator, [ix]);
}
