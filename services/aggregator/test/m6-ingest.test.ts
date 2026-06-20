// M6 ↔ M4 seam: real dual-signed receipts minted by the Rust data plane must verify
// in this aggregator and build a claimable epoch — closing the loop from relayed
// traffic to the on-chain reward merkle.

import { readFileSync } from 'node:fs';
import { type Address } from '@solana/kit';
import { math } from '@weft/sdk';
import { describe, expect, it } from 'vitest';

import { selectReceiptsForEpoch, type TrafficReceipt } from '../src/receipts';
import { buildEpoch, type NodeInfo } from '../src/rewards';

const vectors = JSON.parse(
  readFileSync(new URL('./__fixtures__/receipt-vectors.json', import.meta.url), 'utf8'),
) as { signed: Record<string, string>[] };

function toReceipt(v: Record<string, string>): TrafficReceipt {
  return {
    client: v.client as Address,
    operator: v.operator as Address,
    nodeId: BigInt(v.nodeId),
    bytes: BigInt(v.bytes),
    windowStart: BigInt(v.windowStart),
    windowEnd: BigInt(v.windowEnd),
    nonce: BigInt(v.nonce),
    clientSig: v.clientSig,
    relaySig: v.relaySig,
  };
}

describe('Rust receipts feed the M4 aggregator', () => {
  const receipts = vectors.signed.map(toReceipt);

  it('both dual-signed receipts verify and are in epoch 1', () => {
    const sel = selectReceiptsForEpoch(receipts, 1n);
    expect(sel.accepted.length).toBe(receipts.length);
    expect(sel.rejected.length).toBe(0);
  });

  it('buildEpoch produces a claimable reward tree from the relayed bytes', () => {
    const nodes: NodeInfo[] = vectors.signed.map((v) => ({
      operator: v.operator as Address,
      nodeId: BigInt(v.nodeId),
      reputationBps: 10_000,
      geo: 0,
      stake: 2_000n * math.ONE_WEFT,
    }));
    const build = buildEpoch(1n, receipts, nodes, { minStakeToEarn: 1n });
    expect(build.numNodes).toBe(receipts.length);
    expect(build.totalReward).toBeGreaterThan(0n);
    // each served proof verifies against the posted root.
    const root = math.fromHex(build.root);
    for (const e of build.entries) {
      expect(math.merkleVerify(e.proof.map(math.fromHex), root, math.fromHex(e.leaf))).toBe(true);
    }
  });
});
