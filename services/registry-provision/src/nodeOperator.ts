// Operator-facing "become a node" flow: register a running `weft-node` on-chain (mint its
// cNFT in the node registry) and read back its NodeState. The node's operator key — the same
// ed25519 key the daemon signs its DHT descriptor with — doubles as the Solana signer, so the
// on-chain operator identity and the off-chain node identity are one key. Inputs come from the
// daemon's own outputs: the bootstrap manifest (WEFT_MANIFEST) + the key file (WEFT_OPERATOR_KEY).

import { readFileSync } from 'node:fs';
import {
  createKeyPairSignerFromPrivateKeyBytes,
  getAddressDecoder,
  getAddressEncoder,
  lamports,
  type Address,
  type KeyPairSigner,
} from '@solana/kit';
import { nodeRegistry } from '@weft/sdk';

import { loadEnv, nodePda, type Env } from './config';
import { connect, type Connection } from './kit';
import { loadManifest } from './manifest';
import { registerNode } from './registerNode';

/** The subset of `weft-node`'s `NodeManifest` (WEFT_MANIFEST JSON) the operator flow needs. */
interface NodeManifest {
  operator: string; // hex(32)
  node_id: number;
  geo: number;
  capabilities: number;
  availability: number;
  endpoint_hash: string; // hex(32)
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function fromHex(s: string): Uint8Array {
  const clean = s.trim();
  if (clean.length % 2 !== 0) throw new Error('odd-length hex');
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  return out;
}

function toHex(bytes: ArrayLike<number>): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

export function loadNodeManifest(path: string): NodeManifest {
  return JSON.parse(readFileSync(path, 'utf8')) as NodeManifest;
}

/** The operator's Solana signer, derived from the daemon's 96-byte key file (first 32 bytes
 * = the ed25519 operator seed). Its address equals the manifest's `operator` field. */
export async function operatorSignerFromKeyFile(path: string): Promise<KeyPairSigner> {
  const bytes = fromHex(readFileSync(path, 'utf8'));
  if (bytes.length !== 96) {
    throw new Error(
      `expected a 96-byte hex key (operator‖static‖onion), got ${bytes.length} bytes`,
    );
  }
  return createKeyPairSignerFromPrivateKeyBytes(bytes.slice(0, 32));
}

/** An `Address` from a 32-byte hex public key. */
export function addressFromHex(hex: string): Address {
  return getAddressDecoder().decode(fromHex(hex));
}

/** Ensure `addr` can pay for registration; airdrop on devnet/localnet, else instruct the user. */
async function ensureFunded(conn: Connection, addr: Address, cluster: string): Promise<void> {
  const min = 5_000_000n; // ~0.005 SOL covers the register tx + cNFT rent
  const { value: bal } = await conn.rpc.getBalance(addr).send();
  if (bal >= min) return;
  if (cluster !== 'devnet' && cluster !== 'localnet') {
    throw new Error(`operator ${addr} has ${bal} lamports — fund it (need ≥ ${min}) and retry`);
  }
  console.log(`[node-op] airdropping 1 SOL to ${addr} on ${cluster}…`);
  // `requestAirdrop` is only on the dev/test-cluster RPC API; the generic type omits it.
  const airdropRpc = conn.rpc as unknown as {
    requestAirdrop: (
      a: Address,
      l: ReturnType<typeof lamports>,
    ) => { send: () => Promise<unknown> };
  };
  try {
    await airdropRpc.requestAirdrop(addr, lamports(1_000_000_000n)).send();
  } catch (e) {
    console.warn(`[node-op] airdrop request failed (${(e as Error).message}); polling balance…`);
  }
  for (let i = 0; i < 30; i++) {
    await sleep(2000);
    const { value: b } = await conn.rpc.getBalance(addr).send();
    if (b >= min) {
      console.log(`[node-op] funded: ${b} lamports`);
      return;
    }
  }
  throw new Error(`airdrop did not land; fund ${addr} manually and retry`);
}

export interface RegisterResult {
  signature: string | null;
  alreadyRegistered: boolean;
  operator: Address;
  nodeId: bigint;
  nodeState: Address;
}

/** Register the node described by `manifestPath`, signed + paid for by its own operator key. */
export async function registerFromManifest(
  manifestPath: string,
  keyPath: string,
  env: Env = loadEnv(),
): Promise<RegisterResult> {
  const nm = loadNodeManifest(manifestPath);
  const operator = await operatorSignerFromKeyFile(keyPath);

  // The key file and the manifest must describe the same node.
  const signerHex = toHex(getAddressEncoder().encode(operator.address));
  if (signerHex !== nm.operator.toLowerCase()) {
    throw new Error(
      `key/manifest mismatch: key is ${signerHex} but manifest operator is ${nm.operator}`,
    );
  }

  const conn = connect(env.rpcUrl, env.wsUrl);
  const nodeId = BigInt(nm.node_id);
  const pda = await nodePda(operator.address, nodeId);
  const existing = await fetchNodeStateTolerant(conn, pda);
  if (existing) {
    return {
      signature: null,
      alreadyRegistered: true,
      operator: operator.address,
      nodeId,
      nodeState: pda,
    };
  }

  const reg = loadManifest(env.cluster);
  if (!reg?.complete) {
    throw new Error(`registry not provisioned on ${env.cluster} — run \`pnpm provision\` first`);
  }
  await ensureFunded(conn, operator.address, env.cluster);

  const signature = await registerNode(conn, operator, reg, {
    nodeId,
    geo: nm.geo,
    capabilities: nm.capabilities,
    availability: nm.availability,
    metadataUri: `https://weft.network/node/${nm.node_id}.json`,
    endpointHash: fromHex(nm.endpoint_hash),
  });
  return {
    signature,
    alreadyRegistered: false,
    operator: operator.address,
    nodeId,
    nodeState: pda,
  };
}

export interface NodeStatus {
  registered: boolean;
  nodeState: Address;
  operator: Address;
  nodeId: bigint;
  data?: {
    status: number;
    capabilities: number;
    geo: number;
    availability: number;
    stakeAmount: bigint;
    reputation: number;
    assetId: Address;
    endpointHashHex: string;
    endpointHashMatchesManifest: boolean;
  };
}

/** Decode a NodeState account tolerantly: the `sequence` field was appended in M8, and the
 * program documents that pre-M8 accounts "read 0 here". A strict fixed-size decoder rejects
 * the shorter (pre-`sequence`) layout, so we zero-pad to the expected size — matching the
 * program's own forward-compat semantics — and decode. Returns null if the account is absent
 * or carries the wrong discriminator. */
async function fetchNodeStateTolerant(
  conn: Connection,
  pda: Address,
): Promise<ReturnType<ReturnType<typeof nodeRegistry.getNodeStateDecoder>['decode']> | null> {
  const info = await conn.rpc.getAccountInfo(pda, { encoding: 'base64' }).send();
  if (!info.value) return null;
  let data = Uint8Array.from(Buffer.from(info.value.data[0], 'base64'));
  const disc = nodeRegistry.NODE_STATE_DISCRIMINATOR;
  if (data.length < disc.length || !disc.every((b, i) => data[i] === b)) return null;
  const decoder = nodeRegistry.getNodeStateDecoder();
  if (data.length < decoder.fixedSize) {
    const padded = new Uint8Array(decoder.fixedSize);
    padded.set(data);
    data = padded;
  }
  return decoder.decode(data);
}

/** Read a node's on-chain registration state (and whether its endpoint_hash matches the manifest). */
export async function nodeStatusFromManifest(
  manifestPath: string,
  env: Env = loadEnv(),
): Promise<NodeStatus> {
  const nm = loadNodeManifest(manifestPath);
  const operator = addressFromHex(nm.operator);
  const nodeId = BigInt(nm.node_id);
  const conn = connect(env.rpcUrl, env.wsUrl);
  const pda = await nodePda(operator, nodeId);
  const ns = await fetchNodeStateTolerant(conn, pda);
  if (!ns) {
    return { registered: false, nodeState: pda, operator, nodeId };
  }
  const endpointHashHex = toHex(ns.endpointHash);
  return {
    registered: true,
    nodeState: pda,
    operator,
    nodeId,
    data: {
      status: ns.status,
      capabilities: ns.capabilities,
      geo: ns.geo,
      availability: ns.availability,
      stakeAmount: ns.stakeAmount,
      reputation: ns.reputation,
      assetId: ns.assetId,
      endpointHashHex,
      endpointHashMatchesManifest: endpointHashHex === nm.endpoint_hash.toLowerCase(),
    },
  };
}
