import { describe, expect, it } from 'vitest';

import { fetchNodes, filterNodes, geoRegionPrefix, rpc } from '../src/directory';

const rpcUrl = process.env.WEFT_RPC_URL ?? 'https://api.devnet.solana.com';

describe('node directory (devnet, getProgramAccounts — authoritative, no DAS)', () => {
  it('lists registered nodes from NodeState', async () => {
    const nodes = await fetchNodes(rpc(rpcUrl));
    if (nodes.length === 0) {
      console.warn(
        '[devnet] no NodeState accounts for current program id; run registry rehearsal before asserting node contents',
      );
      return;
    }
    // Validate invariants that hold for ANY registered node (don't assume a specific
    // rehearsal nodeId — live devnet nodes come and go). The key check is that the indexer
    // decodes a real cNFT assetId (not a faked/empty one) and a sane availability.
    for (const n of nodes) {
      expect(n.availability).toBeGreaterThanOrEqual(0);
      expect(n.availability).toBeLessThanOrEqual(100);
      expect(n.assetId).toMatch(/^[1-9A-HJ-NP-Za-km-z]{32,}$/);
    }
  }, 60_000);

  it('filters by capability and availability', async () => {
    const nodes = await fetchNodes(rpc(rpcUrl));
    if (nodes.length === 0) {
      console.warn(
        '[devnet] no NodeState accounts for current program id; run registry rehearsal before asserting filters',
      );
      return;
    }
    expect(
      filterNodes(nodes, { capability: 0b100 }).every((n) => (n.capabilities & 0b100) !== 0),
    ).toBe(true);
    expect(filterNodes(nodes, { minAvailability: 90 }).every((n) => n.availability >= 90)).toBe(
      true,
    );
  }, 60_000);

  it('geoRegionPrefix matches the on-chain packing', () => {
    const geo = 0b11111_00000_11111_00000_11111_00000;
    expect(geoRegionPrefix(geo, 1)).toBe(0b11111);
    expect(geoRegionPrefix(geo, 2)).toBe(0b11111_00000);
  });
});
