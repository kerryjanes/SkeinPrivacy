import { afterEach, describe, expect, it } from 'vitest';
import type { Server } from 'node:http';
import type { Address } from '@solana/kit';

const TEST_TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' as Address;

import { createAggregatorServer } from '../src/server';
import { EpochStore } from '../src/store';
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
        tokenProgram: TEST_TOKEN_PROGRAM,
        label: 'test',
      },
      getBlockhash: async () =>
        ({
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
    expect(body.result).toEqual({
      root: 'abc',
      totalReward: '123',
      numNodes: 1,
      postedSignature: null,
    });
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
        tokenProgram: TEST_TOKEN_PROGRAM,
        label: 'test',
      },
      getBlockhash: async () =>
        ({
          blockhash: '11111111111111111111111111111111',
          lastValidBlockHeight: 1n,
        }) as never,
    });
    const base = await listen(server);

    const res = await fetch(`${base}/claimable?operator=${operator.address}`);
    const body = (await res.json()) as {
      totalAmount: string;
      nodes: Array<{
        nodeId: string;
        totalAmount: string;
        claims: Array<{ epoch: string; amount: string }>;
      }>;
    };

    expect(res.status).toBe(200);
    expect(body.nodes.map((n) => n.nodeId)).toEqual(['11', '12']);
    expect(body.nodes.find((n) => n.nodeId === '11')?.claims.map((c) => c.epoch)).toEqual([
      '8',
      '7',
    ]);
    expect(BigInt(body.totalAmount)).toBeGreaterThan(0n);
  });

  it('rejects unauthenticated /receipts when a receipts token is configured', async () => {
    let ingested = false;
    server = createAggregatorServer({
      store: new EpochStore(),
      receiptsToken: 'relay-secret',
      payConfig: {
        rewardMint: makeSigner().address,
        rewardVault: makeSigner().address,
        treasury: makeSigner().address,
        tokenProgram: TEST_TOKEN_PROGRAM,
        label: 'test',
      },
      getBlockhash: async () =>
        ({ blockhash: '11111111111111111111111111111111', lastValidBlockHeight: 1n }) as never,
      onReceipts: async () => {
        ingested = true;
        return { root: '', totalReward: '0', numNodes: 0, postedSignature: null };
      },
    });
    const base = await listen(server);

    const noAuth = await fetch(`${base}/receipts?epoch=1`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ receipts: [] }),
    });
    expect(noAuth.status).toBe(401);
    expect(ingested).toBe(false);

    const authed = await fetch(`${base}/receipts?epoch=1`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: 'Bearer relay-secret' },
      body: JSON.stringify({ receipts: [] }),
    });
    expect(authed.status).toBe(200);
    expect(ingested).toBe(true);
  });
});
