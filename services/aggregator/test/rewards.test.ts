import { getAddressEncoder } from '@solana/kit';
import { math } from '@weft/sdk';
import { describe, expect, it } from 'vitest';

import { buildEpoch, DEFAULT_MIN_STAKE_TO_EARN, type NodeInfo } from '../src/rewards';
import type { GeoTable } from '../src/geo';
import { makeReceipt, makeSigner } from './helpers';

const addrEnc = getAddressEncoder();
const GB = 1_000_000_000n;
const WEFT = math.ONE_WEFT;

describe('buildEpoch', () => {
  it('rewards a staked node and the proof verifies against the root', () => {
    const client = makeSigner();
    const operator = makeSigner();
    const node: NodeInfo = {
      operator: operator.address,
      nodeId: 1n,
      reputationBps: 10_000, // neutral 1.0x
      geo: 0,
      stake: DEFAULT_MIN_STAKE_TO_EARN, // earns, but below the 10k staking-bonus threshold
    };
    const receipts = [
      makeReceipt(client, operator, {
        nodeId: 1n,
        bytes: GB,
        windowStart: 10n,
        windowEnd: 20n,
        nonce: 1n,
      }),
    ];
    const build = buildEpoch(0n, receipts, [node]);
    expect(build.numNodes).toBe(1);
    // 700 WEFT/GB · 1GB · 1.0x = 700 WEFT
    expect(build.entries[0].amount).toBe(700n * WEFT);
    expect(build.totalReward).toBe(700n * WEFT);

    const leaf = math.hashRewardLeaf(
      0n,
      addrEnc.encode(operator.address) as Uint8Array,
      1n,
      700n * WEFT,
    );
    expect(math.toHex(leaf)).toBe(build.entries[0].leaf);
    const proof = build.entries[0].proof.map(math.fromHex);
    expect(math.merkleVerify(proof, math.fromHex(build.root), leaf)).toBe(true);
  });

  it('skips nodes below the minimum stake and unknown nodes', () => {
    const client = makeSigner();
    const poor = makeSigner();
    const ghost = makeSigner();
    const poorNode: NodeInfo = {
      operator: poor.address,
      nodeId: 1n,
      reputationBps: 10_000,
      geo: 0,
      stake: 1n * WEFT, // below the 1000-WEFT floor
    };
    const receipts = [
      makeReceipt(client, poor, {
        nodeId: 1n,
        bytes: GB,
        windowStart: 1n,
        windowEnd: 2n,
        nonce: 1n,
      }),
      makeReceipt(client, ghost, {
        nodeId: 9n,
        bytes: GB,
        windowStart: 1n,
        windowEnd: 2n,
        nonce: 2n,
      }),
    ];
    const build = buildEpoch(0n, receipts, [poorNode]);
    expect(build.numNodes).toBe(0);
    expect(build.skipped.map((s) => s.reason).sort()).toEqual(['below-min-stake', 'unknown-node']);
    expect(build.root).toBe('');
  });

  it('caps per-node bytes and applies geo + staking bonuses (parity with the mirror)', () => {
    const client = makeSigner();
    const operator = makeSigner();
    // geoRegionPrefix keeps the top chars×5 bits; put region 5 in those bits.
    const region = 5;
    const geo = region << (30 - 10); // top 10 bits (chars=2) = region 5
    const geoTable: GeoTable = { chars: 2, bonusBps: { [String(region)]: 5_000 } };
    const node: NodeInfo = {
      operator: operator.address,
      nodeId: 2n,
      reputationBps: 20_000, // 2.0x
      geo,
      stake: 10_000n * WEFT, // ≥ threshold → +20% staking bonus
    };
    // two receipts summing past the cap
    const receipts = [
      makeReceipt(client, operator, {
        nodeId: 2n,
        bytes: 4n * GB,
        windowStart: 1n,
        windowEnd: 2n,
        nonce: 1n,
      }),
      makeReceipt(client, operator, {
        nodeId: 2n,
        bytes: 4n * GB,
        windowStart: 3n,
        windowEnd: 4n,
        nonce: 2n,
      }),
    ];
    const maxBytes = 5n * GB;
    const build = buildEpoch(0n, receipts, [node], { geoTable, maxBytesPerEpoch: maxBytes });

    expect(build.numNodes).toBe(1);
    const r = build.rewards[0];
    expect(r.bytes).toBe(maxBytes); // capped
    expect(r.geoBonusBps).toBe(5_000);
    expect(r.stakingBonusBps).toBe(2_000);
    const expected = math.trafficReward(maxBytes, 20_000n, 5_000n, 2_000n);
    expect(r.reward).toBe(expected);
  });

  it('builds a multi-node tree where every served proof verifies', () => {
    const client = makeSigner();
    const ops = [makeSigner(), makeSigner(), makeSigner(), makeSigner(), makeSigner()];
    const nodes: NodeInfo[] = ops.map((o, i) => ({
      operator: o.address,
      nodeId: BigInt(i + 1),
      reputationBps: 8_000 + i * 1_000,
      geo: 0,
      stake: 2_000n * WEFT,
    }));
    const receipts = ops.map((o, i) =>
      makeReceipt(client, o, {
        nodeId: BigInt(i + 1),
        bytes: BigInt(i + 1) * GB,
        windowStart: 1n,
        windowEnd: 2n,
        nonce: BigInt(i + 1),
      }),
    );
    const build = buildEpoch(0n, receipts, nodes);
    expect(build.numNodes).toBe(5);
    const root = math.fromHex(build.root);
    for (const e of build.entries) {
      const leaf = math.hashRewardLeaf(
        0n,
        addrEnc.encode(e.operator) as Uint8Array,
        e.nodeId,
        e.amount,
      );
      expect(math.toHex(leaf)).toBe(e.leaf);
      expect(math.merkleVerify(e.proof.map(math.fromHex), root, leaf)).toBe(true);
    }
    // entries are sorted deterministically by operator then node id
    const sorted = [...build.entries].sort((a, b) =>
      a.operator < b.operator ? -1 : a.operator > b.operator ? 1 : 0,
    );
    expect(build.entries.map((e) => e.operator)).toEqual(sorted.map((e) => e.operator));
  });
});

describe('cold-start bootstrap bonus', () => {
  it('applies the bonus to an eligible early node and not to a late one', () => {
    const client = makeSigner();
    const early = makeSigner();
    const late = makeSigner();
    const nodes: NodeInfo[] = [
      {
        operator: early.address,
        nodeId: 1n,
        reputationBps: 10_000,
        geo: 0,
        stake: 2_000n * WEFT,
        sequence: 5n,
      },
      {
        operator: late.address,
        nodeId: 1n,
        reputationBps: 10_000,
        geo: 0,
        stake: 2_000n * WEFT,
        sequence: 50n,
      },
    ];
    const receipts = [
      makeReceipt(client, early, {
        nodeId: 1n,
        bytes: GB,
        windowStart: 1n,
        windowEnd: 2n,
        nonce: 1n,
      }),
      makeReceipt(client, late, {
        nodeId: 1n,
        bytes: GB,
        windowStart: 1n,
        windowEnd: 2n,
        nonce: 2n,
      }),
    ];
    const bootstrap = { nodeLimit: 10n, bonusBps: 5_000n, endTs: 0n };
    const build = buildEpoch(0n, receipts, nodes, { minStakeToEarn: 1n, bootstrap });

    const e = build.rewards.find((r) => r.operator === early.address)!;
    const l = build.rewards.find((r) => r.operator === late.address)!;
    // early (seq 5 <= 10) gets +50%; late (seq 50 > 10) gets nothing.
    expect(e.bootstrapBonusBps).toBe(5_000);
    expect(l.bootstrapBonusBps).toBe(0);
    expect(e.reward).toBe((l.reward * 3n) / 2n);
  });

  it('expires the bonus after the governed end timestamp', () => {
    const client = makeSigner();
    const op = makeSigner();
    const nodes: NodeInfo[] = [
      {
        operator: op.address,
        nodeId: 1n,
        reputationBps: 10_000,
        geo: 0,
        stake: 2_000n * WEFT,
        sequence: 1n,
      },
    ];
    // epoch 1 starts at 600; an endTs of 500 is already past → no bonus.
    const r = makeReceipt(client, op, {
      nodeId: 1n,
      bytes: GB,
      windowStart: 600n,
      windowEnd: 700n,
      nonce: 1n,
    });
    const build = buildEpoch(1n, [r], nodes, {
      minStakeToEarn: 1n,
      bootstrap: { nodeLimit: 10n, bonusBps: 5_000n, endTs: 500n },
    });
    expect(build.rewards[0].bootstrapBonusBps).toBe(0);
  });
});
