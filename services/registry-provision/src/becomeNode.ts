// "Become a node" under your OWN Solana wallet. Registers the device in the
// single Weft core program, signed by the operator wallet, committing the
// public reachable endpoint (relay host:port) on-chain. The same wallet earns
// $WEFT for the traffic this node serves.

import { createHash } from 'node:crypto';
import type { Address, TransactionSigner } from '@solana/kit';
import { nodePda } from './config';
import type { Connection } from './kit';
import { registerNode } from './registerNode';

// Capability bitmask advertised on-chain.
export const CAP_1HOP = 1;
export const CAP_MULTIHOP = 2;
export const CAP_RELAY = 4; // a public node that relays home nodes (frps rendezvous)

export interface BecomeNodeInput {
  /** Public reachable endpoint other users dial — for a home node this is `relayHost:port`. */
  endpoint: string;
  geo?: number; // numeric region code (0 = unspecified)
  capabilities?: number; // default: 1hop | multihop
  availability?: number; // 0..100
}

export interface NodeRegistration {
  signature: string;
  nodeId: bigint;
  node: Address; // the NodeState PDA
  endpoint: string;
}

/** On-chain commitment over the node's public endpoint. */
export function endpointHash(endpoint: string): Uint8Array {
  return new Uint8Array(createHash('sha256').update(endpoint).digest());
}

/** Stable per (operator, endpoint) and fits u64 — re-running for the same endpoint is idempotent. */
export function deriveNodeId(operator: string, endpoint: string): bigint {
  const h = createHash('sha256').update(`${operator}|${endpoint}`).digest();
  let v = 0n;
  for (let i = 0; i < 8; i++) v = (v << 8n) | BigInt(h[i]);
  return v;
}

export async function becomeNode(
  conn: Connection,
  operator: TransactionSigner,
  input: BecomeNodeInput,
  _cluster = 'devnet',
): Promise<NodeRegistration> {
  const nodeId = deriveNodeId(operator.address, input.endpoint);
  const signature = await registerNode(conn, operator, {
    nodeId,
    geo: input.geo ?? 0,
    capabilities: input.capabilities ?? CAP_1HOP,
    availability: input.availability ?? 100,
    endpointHash: endpointHash(input.endpoint),
  });
  const node = await nodePda(operator.address, nodeId);
  return { signature, nodeId, node, endpoint: input.endpoint };
}
