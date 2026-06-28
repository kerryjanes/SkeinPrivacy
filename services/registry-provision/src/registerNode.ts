import type { TransactionSigner } from '@solana/kit';
import { weft } from '@weft/sdk';

import { send, type Connection } from './kit';

export interface NodeParams {
  nodeId: bigint;
  geo: number;
  capabilities: number;
  availability: number;
  /** On-chain commitment over the node's stable identity. Defaults to zeros if omitted. */
  endpointHash?: Uint8Array;
}

/** Register a node through the single Weft core program. */
export async function registerNode(
  conn: Connection,
  operator: TransactionSigner,
  params: NodeParams,
): Promise<string> {
  const ix = await weft.getRegisterNodeInstructionAsync({
    operator,
    nodeId: params.nodeId,
    geo: params.geo,
    capabilities: params.capabilities,
    endpointHash: params.endpointHash ?? new Uint8Array(32),
    availability: params.availability,
  });
  return send(conn, operator, [ix]);
}
