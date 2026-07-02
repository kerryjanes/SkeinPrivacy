import { afterEach, describe, expect, it } from 'vitest';
import type { Server } from 'node:http';
import { ed25519 } from '@noble/curves/ed25519';

import { createAggregatorServer } from '../src/server';
import { EpochStore, PayoutStore } from '../src/store';
import { buildEpochFromByteTotals } from '../src/rewards';
import { makeReceipt, makeSigner } from './helpers';

let server: Server | undefined;

function listen(s: Server): Promise<string> {
  return new Promise((resolve) => {
    s.listen(0, '127.0.0.1', () => {
      const addr = s.address();
      if (!addr || typeof addr === 'string') throw new Error('unexpected server address');
      resolve(`http://127.0.0.1:${addr.port}`);
    });
  });
}

function signMessageBase64(message: string, secretKey: Uint8Array): string {
  return Buffer.from(ed25519.sign(Buffer.from(message, 'utf8'), secretKey)).toString('base64');
}

afterEach(async () => {
  if (!server) return;
  await new Promise<void>((resolve, reject) => {
    server?.close((err) => (err ? reject(err) : resolve()));
  });
  server = undefined;
});

describe('aggregator HTTP receipt ingest', () => {
  it('awaits the production ingest hook and includes its serializable summary', async () => {
    const client = makeSigner();
    const operator = makeSigner();
    const receipt = makeReceipt(client, operator, {
      nodeId: 1n,
      bytes: 100n,
      windowStart: 610n,
      windowEnd: 620n,
      nonce: 1n,
    });

    let seenEpoch = -1n;
    let seenReceipts = 0;
    server = createAggregatorServer({
      store: new EpochStore(),
      payConfig: {
        rewardMint: client.address,
        rewardVault: client.address,
        treasury: client.address,
        label: 'test',
      },
      getBlockhash: async () => ({
        blockhash: '11111111111111111111111111111111',
        lastValidBlockHeight: 1n,
      }) as never,
      onReceipts: async (epoch, accepted) => {
        seenEpoch = epoch;
        seenReceipts = accepted.length;
        return { root: 'abc', totalReward: '123', numNodes: 1, postedSignature: null };
      },
    });
    const base = await listen(server);

    const res = await fetch(`${base}/receipts?epoch=1`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        receipts: [
          {
            ...receipt,
            nodeId: receipt.nodeId.toString(),
            bytes: receipt.bytes.toString(),
            windowStart: receipt.windowStart.toString(),
            windowEnd: receipt.windowEnd.toString(),
            nonce: receipt.nonce.toString(),
          },
        ],
      }),
    });
    const body = (await res.json()) as {
      accepted: number;
      result: { root: string; totalReward: string; numNodes: number };
    };

    expect(res.status).toBe(200);
    expect(body.accepted).toBe(1);
    expect(body.result).toEqual({ root: 'abc', totalReward: '123', numNodes: 1, postedSignature: null });
    expect(seenEpoch).toBe(1n);
    expect(seenReceipts).toBe(1);
  });

  it('summarizes claimable rewards by operator and node without requiring an epoch selector', async () => {
    const operator = makeSigner();
    const other = makeSigner();
    const store = new EpochStore();
    store.put(
      buildEpochFromByteTotals(
        7n,
        [
          { operator: operator.address, nodeId: 11n, bytes: 1_000_000_000n },
          { operator: operator.address, nodeId: 12n, bytes: 2_000_000_000n },
          { operator: other.address, nodeId: 99n, bytes: 3_000_000_000n },
        ],
        [
          { operator: operator.address, nodeId: 11n, reputationBps: 10_000, geo: 0, stake: 0n },
          { operator: operator.address, nodeId: 12n, reputationBps: 10_000, geo: 0, stake: 0n },
          { operator: other.address, nodeId: 99n, reputationBps: 10_000, geo: 0, stake: 0n },
        ],
        { minStakeToEarn: 0n },
      ),
    );
    store.put(
      buildEpochFromByteTotals(
        8n,
        [{ operator: operator.address, nodeId: 11n, bytes: 4_000_000_000n }],
        [{ operator: operator.address, nodeId: 11n, reputationBps: 10_000, geo: 0, stake: 0n }],
        { minStakeToEarn: 0n },
      ),
    );

    server = createAggregatorServer({
      store,
      payConfig: {
        rewardMint: operator.address,
        rewardVault: operator.address,
        treasury: operator.address,
        label: 'test',
      },
      getBlockhash: async () => ({
        blockhash: '11111111111111111111111111111111',
        lastValidBlockHeight: 1n,
      }) as never,
    });
    const base = await listen(server);

    const res = await fetch(`${base}/claimable?operator=${operator.address}`);
    const body = (await res.json()) as {
      totalAmount: string;
      nodes: Array<{ nodeId: string; totalAmount: string; claims: Array<{ epoch: string; amount: string }> }>;
    };

    expect(res.status).toBe(200);
    expect(body.nodes.map((n) => n.nodeId)).toEqual(['11', '12']);
    expect(body.nodes.find((n) => n.nodeId === '11')?.claims.map((c) => c.epoch)).toEqual(['8', '7']);
    expect(BigInt(body.totalAmount)).toBeGreaterThan(0n);
  });

  it('summarizes earned rewards net of off-chain payouts', async () => {
    const operator = makeSigner();
    const store = new EpochStore();
    const payouts = new PayoutStore();
    store.put(
      buildEpochFromByteTotals(
        9n,
        [{ operator: operator.address, nodeId: 11n, bytes: 1_000_000_000n }],
        [{ operator: operator.address, nodeId: 11n, reputationBps: 10_000, geo: 0, stake: 0n }],
        { minStakeToEarn: 0n },
      ),
    );
    payouts.record(operator.address, 11n, 100n, 'sig-1', 1);

    server = createAggregatorServer({
      store,
      payoutStore: payouts,
      payConfig: {
        rewardMint: operator.address,
        rewardVault: operator.address,
        treasury: operator.address,
        label: 'test',
      },
      getBlockhash: async () => ({
        blockhash: '11111111111111111111111111111111',
        lastValidBlockHeight: 1n,
      }) as never,
    });
    const base = await listen(server);

    const res = await fetch(`${base}/earned?operator=${operator.address}`);
    const body = (await res.json()) as {
      totalEarned: string;
      totalPaid: string;
      withdrawable: string;
      nodes: Array<{ nodeId: string; earned: string; paid: string; withdrawable: string }>;
    };

    expect(res.status).toBe(200);
    expect(body.nodes).toHaveLength(1);
    expect(BigInt(body.totalEarned)).toBe(700n * 1_000_000n);
    expect(body.totalPaid).toBe('100');
    expect(body.withdrawable).toBe((700n * 1_000_000n - 100n).toString());
    expect(body.nodes[0]).toMatchObject({ nodeId: '11', paid: '100' });
  });

  it('requires an operator-signed challenge before withdrawing earned rewards', async () => {
    const operator = makeSigner();
    const store = new EpochStore();
    const payouts = new PayoutStore();
    store.put(
      buildEpochFromByteTotals(
        10n,
        [{ operator: operator.address, nodeId: 11n, bytes: 1_000_000_000n }],
        [{ operator: operator.address, nodeId: 11n, reputationBps: 10_000, geo: 0, stake: 0n }],
        { minStakeToEarn: 0n },
      ),
    );

    server = createAggregatorServer({
      store,
      payoutStore: payouts,
      payout: {
        availableBalance: async () => 1_000_000_000_000_000n,
        pay: async (recipient, amount) => ({ signature: `paid-${recipient}-${amount}` }),
      },
      payConfig: {
        rewardMint: operator.address,
        rewardVault: operator.address,
        treasury: operator.address,
        label: 'test',
      },
      getBlockhash: async () => ({
        blockhash: '11111111111111111111111111111111',
        lastValidBlockHeight: 1n,
      }) as never,
    });
    const base = await listen(server);

    const unsigned = await fetch(`${base}/withdraw-earned`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ operator: operator.address }),
    });
    expect(unsigned.status).toBe(401);

    const challengeRes = await fetch(`${base}/withdraw-earned/challenge`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ operator: operator.address }),
    });
    const challenge = (await challengeRes.json()) as {
      message: string;
      expiresAt: number;
      nonce: string;
    };
    expect(challengeRes.status).toBe(200);
    expect(challenge.message).toContain(`operator: ${operator.address}`);
    expect(challenge.message).toContain('nodeId: all');

    const invalid = await fetch(`${base}/withdraw-earned`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        operator: operator.address,
        message: challenge.message,
        signature: Buffer.alloc(64).toString('base64'),
      }),
    });
    expect(invalid.status).toBe(401);

    const signature = signMessageBase64(challenge.message, operator.secretKey);
    const res = await fetch(`${base}/withdraw-earned`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ operator: operator.address, message: challenge.message, signature }),
    });
    const body = (await res.json()) as { signature: string; amount: string };

    expect(res.status).toBe(200);
    expect(body.amount).toBe((700n * 1_000_000n).toString());
    expect(body.signature).toContain('paid-');
    expect(payouts.paid(operator.address, 11n)).toBe(700n * 1_000_000n);
  });

  it('refuses earned withdrawals when payout wallet balance cannot cover amount plus reserve', async () => {
    const operator = makeSigner();
    const store = new EpochStore();
    const payouts = new PayoutStore();
    store.put(
      buildEpochFromByteTotals(
        11n,
        [{ operator: operator.address, nodeId: 11n, bytes: 1_000_000_000n }],
        [{ operator: operator.address, nodeId: 11n, reputationBps: 10_000, geo: 0, stake: 0n }],
        { minStakeToEarn: 0n },
      ),
    );
    let paid = false;

    server = createAggregatorServer({
      store,
      payoutStore: payouts,
      payoutReserve: 10n,
      payout: {
        availableBalance: async () => 500n * 1_000_000n, // < 700 WEFT earned → cannot cover
        pay: async () => {
          paid = true;
          return { signature: 'should-not-pay' };
        },
      } as never,
      payConfig: {
        rewardMint: operator.address,
        rewardVault: operator.address,
        treasury: operator.address,
        label: 'test',
      },
      getBlockhash: async () => ({
        blockhash: '11111111111111111111111111111111',
        lastValidBlockHeight: 1n,
      }) as never,
    });
    const base = await listen(server);
    const challengeRes = await fetch(`${base}/withdraw-earned/challenge`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ operator: operator.address }),
    });
    const challenge = (await challengeRes.json()) as { message: string };
    const signature = signMessageBase64(challenge.message, operator.secretKey);

    const res = await fetch(`${base}/withdraw-earned`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ operator: operator.address, message: challenge.message, signature }),
    });
    const body = (await res.json()) as { error?: string };

    expect(res.status).toBe(503);
    expect(body.error).toMatch(/payout wallet balance/i);
    expect(paid).toBe(false);
    expect(payouts.paid(operator.address, 11n)).toBe(0n);
  });
});
