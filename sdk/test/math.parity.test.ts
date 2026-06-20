// Cross-language parity: the TS reward/merkle mirror must reproduce, byte for
// byte, the golden vectors emitted by the Rust source of truth
// (`cargo run -p weft-primitives --example golden`). If this drifts, the
// aggregator would post amounts/roots the on-chain `claim` rejects.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';

import {
  fromHex,
  hashAllocationLeaf,
  hashRewardLeaf,
  merkleProof,
  merkleRoot,
  merkleVerify,
  splitPayment,
  splitTge,
  toHex,
  trafficReward,
  trafficRewardWithBootstrap,
} from '../src/math';

const here = dirname(fileURLToPath(import.meta.url));
const vectors = JSON.parse(
  readFileSync(join(here, '..', 'src', '__fixtures__', 'reward-vectors.json'), 'utf8'),
) as {
  rewards: {
    bytes: string;
    reputationBps: number;
    geoBonusBps: number;
    stakingBonusBps: number;
    reward: string;
  }[];
  splits: { amount: string; nodes: string; burn: string; treasury: string }[];
  bootstrap: {
    bytes: string;
    reputationBps: number;
    geoBonusBps: number;
    stakingBonusBps: number;
    bootstrapBonusBps: number;
    reward: string;
  }[];
  tge: { allocation: string; tgeBps: number; tge: string; vesting: string }[];
  allocLeaves: { distributor: string; claimant: string; amount: string; leaf: string }[];
  leaves: { epoch: number; operator: string; nodeId: string; amount: string; leaf: string }[];
  tree: {
    epoch: number;
    root: string;
    entries: { operator: string; nodeId: string; amount: string; proof: string[] }[];
  };
};

describe('trafficReward parity', () => {
  it.each(vectors.rewards)(
    'bytes=$bytes rep=$reputationBps geo=$geoBonusBps stk=$stakingBonusBps',
    (v) => {
      const got = trafficReward(
        BigInt(v.bytes),
        BigInt(v.reputationBps),
        BigInt(v.geoBonusBps),
        BigInt(v.stakingBonusBps),
      );
      expect(got.toString()).toBe(v.reward);
    },
  );
});

describe('splitPayment parity', () => {
  it.each(vectors.splits)('amount=$amount', (v) => {
    const s = splitPayment(BigInt(v.amount));
    expect(s.nodes.toString()).toBe(v.nodes);
    expect(s.burn.toString()).toBe(v.burn);
    expect(s.treasury.toString()).toBe(v.treasury);
    // invariant: the split is exact
    expect(s.nodes + s.burn + s.treasury).toBe(BigInt(v.amount));
  });
});

describe('trafficRewardWithBootstrap parity', () => {
  it.each(vectors.bootstrap)('bytes=$bytes boot=$bootstrapBonusBps', (v) => {
    const got = trafficRewardWithBootstrap(
      BigInt(v.bytes),
      BigInt(v.reputationBps),
      BigInt(v.geoBonusBps),
      BigInt(v.stakingBonusBps),
      BigInt(v.bootstrapBonusBps),
    );
    expect(got.toString()).toBe(v.reward);
  });
});

describe('splitTge parity', () => {
  it.each(vectors.tge)('allocation=$allocation tge=$tgeBps', (v) => {
    const s = splitTge(BigInt(v.allocation), BigInt(v.tgeBps));
    expect(s.tge.toString()).toBe(v.tge);
    expect(s.vesting.toString()).toBe(v.vesting);
    expect(s.tge + s.vesting).toBe(BigInt(v.allocation));
  });
});

describe('hashAllocationLeaf parity', () => {
  it.each(vectors.allocLeaves)('claimant=$claimant', (v) => {
    const leaf = hashAllocationLeaf(fromHex(v.distributor), fromHex(v.claimant), BigInt(v.amount));
    expect(toHex(leaf)).toBe(v.leaf);
  });
});

describe('hashRewardLeaf parity', () => {
  it.each(vectors.leaves)('epoch=$epoch node=$nodeId', (v) => {
    const leaf = hashRewardLeaf(
      BigInt(v.epoch),
      fromHex(v.operator),
      BigInt(v.nodeId),
      BigInt(v.amount),
    );
    expect(toHex(leaf)).toBe(v.leaf);
  });

  it('matches the on-chain golden vector', () => {
    const leaf = hashRewardLeaf(42n, new Uint8Array(32).fill(2), 7n, 123_456n);
    expect(toHex(leaf)).toBe('cea5f73a341abd012da25e67633b67f133416f1c5d045b877b8b9602bd8424f1');
  });
});

describe('merkle tree parity', () => {
  const { tree } = vectors;
  const leaves = tree.entries.map((e) =>
    hashRewardLeaf(BigInt(tree.epoch), fromHex(e.operator), BigInt(e.nodeId), BigInt(e.amount)),
  );

  it('root matches the Rust builder', () => {
    expect(toHex(merkleRoot(leaves))).toBe(tree.root);
  });

  it.each(tree.entries.map((e, i) => ({ ...e, i })))('proof[$i] verifies', (e) => {
    const proof = merkleProof(leaves, e.i);
    // the proof we build matches the Rust-emitted proof…
    expect(proof.map(toHex)).toEqual(e.proof);
    // …and verifies against the root the same way the on-chain program checks it
    expect(merkleVerify(proof, fromHex(tree.root), leaves[e.i])).toBe(true);
  });

  it('rejects a tampered proof', () => {
    const proof = merkleProof(leaves, 0);
    const bad = proof.map((p) => Uint8Array.from(p));
    bad[0][0] ^= 1;
    expect(merkleVerify(bad, fromHex(tree.root), leaves[0])).toBe(false);
  });
});
