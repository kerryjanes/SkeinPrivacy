import { afterEach, describe, expect, it } from 'vitest';
import type { Server } from 'node:http';

import { createAggregatorServer } from '../src/server';
import { EpochStore } from '../src/store';
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
});
