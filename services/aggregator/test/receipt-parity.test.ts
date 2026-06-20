// Cross-language receipt parity: the TS `encodeReceiptCore` must reproduce, byte for
// byte, the 104-byte preimage emitted by the Rust data plane
// (`cargo run -p weft-net --example receipt_golden`). If this drifts, a Rust-minted
// receipt would not verify in this aggregator (or on-chain).

import { readFileSync } from 'node:fs';
import { address } from '@solana/kit';
import { describe, expect, it } from 'vitest';

import { encodeReceiptCore } from '../src/receipts';

const vectors = JSON.parse(
  readFileSync(new URL('./__fixtures__/receipt-vectors.json', import.meta.url), 'utf8'),
) as { parity: Record<string, string>[] };

function toHex(b: Uint8Array): string {
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
}

describe('receipt-core parity (Rust ↔ TS)', () => {
  it.each(vectors.parity)('client=$client node=$nodeId', (v) => {
    const core = encodeReceiptCore({
      client: address(v.client),
      operator: address(v.operator),
      nodeId: BigInt(v.nodeId),
      bytes: BigInt(v.bytes),
      windowStart: BigInt(v.windowStart),
      windowEnd: BigInt(v.windowEnd),
      nonce: BigInt(v.nonce),
    });
    expect(core.length).toBe(104);
    expect(toHex(core)).toBe(v.core);
  });
});
