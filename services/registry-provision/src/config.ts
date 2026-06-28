// Config: env and kit PDA derivations for the single Weft core program.

import { readFileSync } from 'node:fs';
import { getAddressEncoder, getProgramDerivedAddress, type Address } from '@solana/kit';
import { weft } from '@weft/sdk';

export const NODE_REGISTRY_PROGRAM = weft.WEFT_PROGRAM_ADDRESS;

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
  if (cluster.startsWith('mainnet') && !overrides.rpcUrl && !process.env.WEFT_RPC_URL) {
    throw new Error(`WEFT_RPC_URL must be set explicitly for ${cluster}`);
  }
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

export async function nodePda(operator: Address, nodeId: bigint): Promise<Address> {
  const id = new Uint8Array(8);
  new DataView(id.buffer).setBigUint64(0, nodeId, true);
  const [pda] = await getProgramDerivedAddress({
    programAddress: NODE_REGISTRY_PROGRAM,
    seeds: [new TextEncoder().encode('node'), addrEnc.encode(operator), id],
  });
  return pda;
}
