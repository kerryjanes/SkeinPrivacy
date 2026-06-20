// Dual-signed traffic receipts. A receipt is metering evidence that a `client`
// consumed `bytes` from an `operator`'s node during a time window; BOTH the
// client and the relay (operator) sign the same canonical core, so neither side
// can unilaterally fabricate billable traffic. The aggregator verifies both
// ed25519 signatures, dedupes by (operator, nonce), and drops out-of-epoch
// windows before any reward is computed.

import { ed25519 } from '@noble/curves/ed25519';
import { getAddressEncoder, type Address } from '@solana/kit';
import { math } from '@weft/sdk';

import { windowInEpoch } from './epoch';

const addrEnc = getAddressEncoder();

export interface TrafficReceiptCore {
  client: Address;
  operator: Address;
  nodeId: bigint;
  bytes: bigint;
  windowStart: bigint;
  windowEnd: bigint;
  nonce: bigint;
}

export interface TrafficReceipt extends TrafficReceiptCore {
  /** ed25519 over `encodeReceiptCore`, by the client's key (hex). */
  clientSig: string;
  /** ed25519 over `encodeReceiptCore`, by the operator's (relay) key (hex). */
  relaySig: string;
}

function le64(v: bigint): Uint8Array {
  const out = new Uint8Array(8);
  let x = v & ((1n << 64n) - 1n);
  for (let i = 0; i < 8; i++) {
    out[i] = Number(x & 0xffn);
    x >>= 8n;
  }
  return out;
}

/**
 * Canonical signing preimage:
 * `client(32) ‖ operator(32) ‖ nodeId_le ‖ bytes_le ‖ windowStart_le ‖ windowEnd_le ‖ nonce_le`.
 */
export function encodeReceiptCore(c: TrafficReceiptCore): Uint8Array {
  const parts = [
    addrEnc.encode(c.client) as Uint8Array,
    addrEnc.encode(c.operator) as Uint8Array,
    le64(c.nodeId),
    le64(c.bytes),
    le64(c.windowStart),
    le64(c.windowEnd),
    le64(c.nonce),
  ];
  const len = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(len);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

/** Sign a receipt core with a raw 32-byte ed25519 secret key; returns hex. */
export function signReceiptCore(core: TrafficReceiptCore, secretKey: Uint8Array): string {
  return math.toHex(ed25519.sign(encodeReceiptCore(core), secretKey));
}

/** Both signatures must verify against the client and operator addresses. */
export function verifyReceipt(r: TrafficReceipt): boolean {
  const msg = encodeReceiptCore(r);
  try {
    const clientPk = addrEnc.encode(r.client) as Uint8Array;
    const operatorPk = addrEnc.encode(r.operator) as Uint8Array;
    return (
      ed25519.verify(math.fromHex(r.clientSig), msg, clientPk) &&
      ed25519.verify(math.fromHex(r.relaySig), msg, operatorPk)
    );
  } catch {
    return false;
  }
}

export type RejectReason = 'bad-signature' | 'out-of-epoch' | 'duplicate-nonce';

export interface ReceiptSelection {
  accepted: TrafficReceipt[];
  rejected: { receipt: TrafficReceipt; reason: RejectReason }[];
}

/**
 * Verify, epoch-filter, and dedupe a batch of receipts for one epoch. Dedup key
 * is `(operator, nonce)` — a nonce is unique per relay, so a client can't replay
 * the same operator receipt, and the first-seen wins.
 */
export function selectReceiptsForEpoch(
  receipts: TrafficReceipt[],
  epoch: bigint,
): ReceiptSelection {
  const accepted: TrafficReceipt[] = [];
  const rejected: { receipt: TrafficReceipt; reason: RejectReason }[] = [];
  const seen = new Set<string>();
  for (const r of receipts) {
    if (!verifyReceipt(r)) {
      rejected.push({ receipt: r, reason: 'bad-signature' });
      continue;
    }
    if (!windowInEpoch(epoch, r.windowStart, r.windowEnd)) {
      rejected.push({ receipt: r, reason: 'out-of-epoch' });
      continue;
    }
    const key = `${r.operator}:${r.nonce}`;
    if (seen.has(key)) {
      rejected.push({ receipt: r, reason: 'duplicate-nonce' });
      continue;
    }
    seen.add(key);
    accepted.push(r);
  }
  return { accepted, rejected };
}
