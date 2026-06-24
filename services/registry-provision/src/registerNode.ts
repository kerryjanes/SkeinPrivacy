import { address, type TransactionSigner } from '@solana/kit';
import { nodeRegistry } from '@weft/sdk';

import { PROGRAMS, treeConfigPda } from './config';
import { send, type Connection } from './kit';
import type { Manifest } from './manifest';

export interface NodeParams {
  nodeId: bigint;
  geo: number;
  capabilities: number;
  availability: number;
  metadataUri: string;
  /** On-chain commitment over the node's stable identity. Defaults to zeros if omitted. */
  endpointHash?: Uint8Array;
}

/** Register a node through the node-registry program (CPIs Bubblegum mintV2). */
export async function registerNode(
  conn: Connection,
  operator: TransactionSigner,
  manifest: Manifest,
  params: NodeParams,
): Promise<string> {
  const merkleTree = address(manifest.merkleTree);
  const treeConfig = await treeConfigPda(merkleTree);
  const ix = await nodeRegistry.getRegisterInstructionAsync({
    operator,
    treeShard: address(manifest.treeShard),
    treeConfig,
    merkleTree,
    coreCollection: address(manifest.collection),
    mplCoreCpiSigner: PROGRAMS.mplCoreCpiSigner,
    logWrapper: PROGRAMS.mplNoop,
    compressionProgram: PROGRAMS.mplAccountCompression,
    mplCoreProgram: PROGRAMS.mplCore,
    bubblegumProgram: PROGRAMS.bubblegum,
    nodeId: params.nodeId,
    geo: params.geo,
    capabilities: params.capabilities,
    endpointHash: params.endpointHash ?? new Uint8Array(32),
    availability: params.availability,
    metadataUri: params.metadataUri,
  });
  return send(conn, operator, [ix]);
}
