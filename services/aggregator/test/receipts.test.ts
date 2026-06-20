import { describe, expect, it } from 'vitest';

import { selectReceiptsForEpoch, verifyReceipt } from '../src/receipts';
import { makeReceipt, makeSigner } from './helpers';

describe('dual-signed receipts', () => {
  it('verifies a well-formed receipt', () => {
    const client = makeSigner();
    const operator = makeSigner();
    const r = makeReceipt(client, operator, {
      nodeId: 1n,
      bytes: 1_000_000_000n,
      windowStart: 10n,
      windowEnd: 20n,
      nonce: 1n,
    });
    expect(verifyReceipt(r)).toBe(true);
  });

  it('rejects a tampered receipt (bytes changed after signing)', () => {
    const client = makeSigner();
    const operator = makeSigner();
    const r = makeReceipt(client, operator, {
      nodeId: 1n,
      bytes: 1_000_000_000n,
      windowStart: 10n,
      windowEnd: 20n,
      nonce: 1n,
    });
    expect(verifyReceipt({ ...r, bytes: 9_999_999_999n })).toBe(false);
  });

  it('rejects when only one party signed (forged relay sig)', () => {
    const client = makeSigner();
    const operator = makeSigner();
    const attacker = makeSigner();
    const r = makeReceipt(client, attacker, {
      nodeId: 1n,
      bytes: 1_000_000_000n,
      windowStart: 10n,
      windowEnd: 20n,
      nonce: 1n,
    });
    // claim it was operator's node, but it was signed by the attacker
    expect(verifyReceipt({ ...r, operator: operator.address })).toBe(false);
  });

  it('selects only in-epoch, deduped, valid receipts', () => {
    const client = makeSigner();
    const operator = makeSigner();
    const inEpoch = makeReceipt(client, operator, {
      nodeId: 1n,
      bytes: 100n,
      windowStart: 10n,
      windowEnd: 20n,
      nonce: 1n,
    });
    const dupNonce = makeReceipt(client, operator, {
      nodeId: 1n,
      bytes: 200n,
      windowStart: 30n,
      windowEnd: 40n,
      nonce: 1n, // same (operator, nonce) → duplicate
    });
    const outOfEpoch = makeReceipt(client, operator, {
      nodeId: 1n,
      bytes: 300n,
      windowStart: 700n, // epoch 1, not epoch 0
      windowEnd: 750n,
      nonce: 2n,
    });
    const badSig = makeReceipt(client, operator, {
      nodeId: 1n,
      bytes: 400n,
      windowStart: 50n,
      windowEnd: 60n,
      nonce: 3n,
    });
    badSig.clientSig = badSig.clientSig.replace(/^../, '00');

    const sel = selectReceiptsForEpoch([inEpoch, dupNonce, outOfEpoch, badSig], 0n);
    expect(sel.accepted).toHaveLength(1);
    expect(sel.accepted[0].nonce).toBe(1n);
    expect(sel.rejected.map((r) => r.reason).sort()).toEqual([
      'bad-signature',
      'duplicate-nonce',
      'out-of-epoch',
    ]);
  });
});
