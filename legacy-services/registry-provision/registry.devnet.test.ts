// Devnet end-to-end: provision (idempotent) → register a node cNFT via the
// program's Bubblegum mintV2 CPI → verify on-chain. The cNFT mint is proven by
// matching NodeState.assetId to Bubblegum's canonical leaf asset-id derivation
// (independent of any indexer). A DAS lookup is attempted as a soft check —
// Helius devnet does not yet index Bubblegum V2 trees, and NodeState is the
// authoritative directory source, so a DAS miss does not fail the milestone.

import { beforeAll, describe, expect, it } from 'vitest';
import { nodeRegistry } from '@weft/sdk';

import { assetIdPda, loadEnv, nodePda } from '../src/config';
import { connect, loadSigner } from '../src/kit';
import { provision } from '../src/provision';
import { registerNode } from '../src/registerNode';
import type { Manifest } from '../src/manifest';

const env = loadEnv({ cluster: 'devnet' });
const NODE_ID = 1n;

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function dasGetAsset(rpcUrl: string, assetId: string): Promise<any> {
  const res = await fetch(rpcUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'getAsset', params: { id: assetId } }),
  });
  return (await res.json()).result;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('node registry on devnet', () => {
  let manifest: Manifest;
  let operator: Awaited<ReturnType<typeof loadSigner>>;
  let node: Awaited<ReturnType<typeof nodeRegistry.fetchNodeState>>;

  beforeAll(async () => {
    manifest = await provision(env);
    const conn = connect(env.rpcUrl, env.wsUrl);
    operator = await loadSigner(env.keypairPath);

    const pda = await nodePda(operator.address, NODE_ID);
    const existing = await nodeRegistry.fetchMaybeNodeState(conn.rpc, pda);
    if (!existing.exists) {
      await registerNode(conn, operator, manifest, {
        nodeId: NODE_ID,
        geo: 0b11111_00000_11111_00000_11111_00000,
        capabilities: 0b000101, // WIREGUARD | EXIT
        availability: 95,
        metadataUri: 'https://weft.network/node/1.json',
      });
      await sleep(2000);
    }
    node = await nodeRegistry.fetchNodeState(conn.rpc, pda);
  }, 120_000);

  it('records NodeState with correct fields', () => {
    expect(node.data.operator).toBe(operator.address);
    expect(node.data.availability).toBe(95);
    expect(node.data.status).toBe(0); // active
    expect(node.data.capabilities).toBe(5); // WIREGUARD | EXIT
  });

  it('mints the cNFT: NodeState.assetId matches the canonical Bubblegum leaf id', async () => {
    const expected = await assetIdPda(manifest.merkleTree as never, node.data.leafNonce);
    // Proves the program minted a real cNFT at this leaf with the canonical
    // asset id (the register tx would have rolled back if the CPI had failed).
    expect(node.data.assetId).toBe(expected);
  });

  it('(soft) appears in DAS where Bubblegum V2 indexing is available', async () => {
    const asset = await dasGetAsset(env.rpcUrl, node.data.assetId);
    if (!asset?.id) {
      console.warn(
        `[das] asset ${node.data.assetId} not indexed (Helius devnet lacks ` +
          `Bubblegum V2 indexing); NodeState is authoritative.`,
      );
      return;
    }
    expect(asset.ownership.owner).toBe(operator.address);
    expect(asset.compression.compressed).toBe(true);
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const coll = (asset.grouping ?? []).find((g: any) => g.group_key === 'collection');
    expect(coll?.group_value).toBe(manifest.collection);
  }, 30_000);
});
