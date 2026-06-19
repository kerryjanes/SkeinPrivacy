// Config: program IDs, env, and kit PDA derivations for the node registry.

import { readFileSync } from 'node:fs';
import { address, getAddressEncoder, getProgramDerivedAddress, type Address } from '@solana/kit';
import { nodeRegistry } from '@weft/sdk';

// Bubblegum V2 stack (resolved from the umi mintV2 defaults).
export const PROGRAMS = {
  bubblegum: address('BGUMAp9Gq7iTEuizy4pqaxsTyUCBK68MDfK752saRPUY'),
  mplCore: address('CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d'),
  mplNoop: address('mnoopTCrg4p8ry25e4bcWA9XZjbNjMTfgYVGGEdRsf3'),
  mplAccountCompression: address('mcmt6YrQEMKw8Mw43FmpRLmf7BqRnFMKmAcbxE3xkAW'),
  mplCoreCpiSigner: address('CbNY3JiXdXNE9tPNEk1aRZVEkWdj2v7kfJLNQwZZgpXk'),
} as const;

export const NODE_REGISTRY_PROGRAM = nodeRegistry.NODE_REGISTRY_PROGRAM_ADDRESS;
export const TREE_MAX_DEPTH = 14;
export const TREE_MAX_BUFFER = 64;

export interface Env {
  cluster: string;
  rpcUrl: string;
  wsUrl: string;
  keypairPath: string;
}

export function deriveWsUrl(rpcUrl: string): string {
  const u = new URL(rpcUrl);
  u.protocol = u.protocol === 'https:' ? 'wss:' : 'ws:';
  if (u.port === '8899') u.port = '8900';
  return u.toString().replace(/\/$/, '');
}

export function loadEnv(overrides: Partial<Env> = {}): Env {
  const cluster = overrides.cluster ?? process.env.WEFT_CLUSTER ?? 'devnet';
  const rpcUrl =
    overrides.rpcUrl ??
    process.env.WEFT_RPC_URL ??
    (cluster === 'devnet' ? 'https://api.devnet.solana.com' : 'http://127.0.0.1:8899');
  return {
    cluster,
    rpcUrl,
    wsUrl: overrides.wsUrl ?? deriveWsUrl(rpcUrl),
    keypairPath:
      overrides.keypairPath ??
      process.env.WEFT_KEYPAIR ??
      `${process.env.HOME}/.config/solana/id.json`,
  };
}

export function loadKeypairBytes(path: string): Uint8Array {
  return Uint8Array.from(JSON.parse(readFileSync(path, 'utf8')) as number[]);
}

const addrEnc = getAddressEncoder();

export async function registryPda(): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: NODE_REGISTRY_PROGRAM,
    seeds: [new TextEncoder().encode('registry')],
  });
  return pda;
}

export async function treeShardPda(index: number): Promise<Address> {
  const idx = new Uint8Array(2);
  new DataView(idx.buffer).setUint16(0, index, true);
  const [pda] = await getProgramDerivedAddress({
    programAddress: NODE_REGISTRY_PROGRAM,
    seeds: [new TextEncoder().encode('tree'), idx],
  });
  return pda;
}

export async function treeConfigPda(merkleTree: Address): Promise<Address> {
  const [pda] = await getProgramDerivedAddress({
    programAddress: PROGRAMS.bubblegum,
    seeds: [addrEnc.encode(merkleTree)],
  });
  return pda;
}

export async function nodePda(operator: Address, nodeId: bigint): Promise<Address> {
  const id = new Uint8Array(8);
  new DataView(id.buffer).setBigUint64(0, nodeId, true);
  const [pda] = await getProgramDerivedAddress({
    programAddress: NODE_REGISTRY_PROGRAM,
    seeds: [new TextEncoder().encode('node'), addrEnc.encode(operator), id],
  });
  return pda;
}

export async function assetIdPda(merkleTree: Address, nonce: bigint): Promise<Address> {
  const n = new Uint8Array(8);
  new DataView(n.buffer).setBigUint64(0, nonce, true);
  const [pda] = await getProgramDerivedAddress({
    programAddress: PROGRAMS.bubblegum,
    seeds: [new TextEncoder().encode('asset'), addrEnc.encode(merkleTree), n],
  });
  return pda;
}
