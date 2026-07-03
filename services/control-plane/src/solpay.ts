// Optional SOL payment path (off by default; enabled with WEFT_SOL_PAYMENTS=1).
//
// Users send SOL to a collection wallet; this module watches that wallet, attributes each
// incoming SystemProgram transfer to its sender, and credits that wallet a prepaid-bytes
// allowance (SOL / price-per-GB). The Controller ADDS this allowance on top of the escrow
// quota — when this module is absent, quota is exactly the escrow quota (unchanged).
//
// The collected SOL is converted to $WEFT and burned by the separate buyback-burn worker.
// Idempotent: a persistent cursor + processed-signature window credit each tx exactly once,
// so restarts never double-credit. On first enable it snaps the cursor to the latest sig so
// historical transfers are not retroactively credited.

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { address } from '@solana/kit';
import { math } from '@weft/sdk';
import type { Rpc } from './chain.js';

const SYSTEM_PROGRAM = '11111111111111111111111111111111';

export interface SolPayConfig {
  collection: string; // pubkey that receives user SOL
  pricePerGbLamports: bigint; // SOL (lamports) a user pays per GB of quota
  minLamports: bigint; // ignore dust/spam below this
  storePath: string;
}

interface Ledger {
  creditedBytes: Record<string, string>; // wallet -> prepaid bytes (bigint as string)
  cursor: string | null; // newest signature already accounted for
  processed: string[]; // recent signatures (dedupe window)
}

function loadLedger(path: string): Ledger {
  if (existsSync(path)) {
    try {
      const j = JSON.parse(readFileSync(path, 'utf8'));
      return {
        creditedBytes: j.creditedBytes ?? {},
        cursor: j.cursor ?? null,
        processed: Array.isArray(j.processed) ? j.processed : [],
      };
    } catch {
      /* fall through */
    }
  }
  return { creditedBytes: {}, cursor: null, processed: [] };
}

export class SolPay {
  private ledger: Ledger;

  constructor(private cfg: SolPayConfig) {
    this.ledger = loadLedger(cfg.storePath);
  }

  /** Prepaid byte allowance a wallet has bought with SOL (0 if none). */
  prepaidBytes(wallet: string): bigint {
    return BigInt(this.ledger.creditedBytes[wallet] ?? '0');
  }

  private save(): void {
    // keep the dedupe window bounded
    if (this.ledger.processed.length > 1000) {
      this.ledger.processed = this.ledger.processed.slice(-1000);
    }
    writeFileSync(this.cfg.storePath, JSON.stringify(this.ledger, null, 2));
  }

  /** Parse a tx for a SystemProgram transfer into the collection wallet. */
  private async parseTransfer(
    r: Rpc,
    signature: string,
  ): Promise<{ sender: string; lamports: bigint } | null> {
    const tx = await r
      .getTransaction(signature as Parameters<Rpc['getTransaction']>[0], {
        commitment: 'confirmed',
        maxSupportedTransactionVersion: 0,
        encoding: 'json',
      })
      .send();
    if (!tx || tx.meta?.err) return null;
    const msg = tx.transaction.message;
    const keys = msg.accountKeys.map((k) => String(k));
    for (const ix of msg.instructions) {
      if (keys[ix.programIdIndex] !== SYSTEM_PROGRAM) continue;
      const data = bs58Decode(ix.data);
      // SystemProgram::Transfer = u32 LE tag 2, then u64 LE lamports (12 bytes total).
      if (data.length !== 12 || data[0] !== 2 || data[1] !== 0 || data[2] !== 0 || data[3] !== 0) {
        continue;
      }
      const from = keys[ix.accounts[0]];
      const to = keys[ix.accounts[1]];
      if (to !== this.cfg.collection) continue;
      return { sender: from, lamports: readU64LE(data, 4) };
    }
    return null;
  }

  /** Incrementally credit new SOL payments. Cheap when idle (one getSignaturesForAddress). */
  async poll(r: Rpc): Promise<void> {
    const sigs = await r
      .getSignaturesForAddress(address(this.cfg.collection), {
        limit: 100,
        ...(this.ledger.cursor ? { until: this.ledger.cursor as never } : {}),
      })
      .send();
    if (sigs.length === 0) return;
    const newest = sigs[0].signature;

    // First enable: snap the cursor forward without crediting history.
    if (this.ledger.cursor === null) {
      this.ledger.cursor = newest;
      this.save();
      return;
    }

    // sigs are newest-first; credit oldest-first.
    for (const s of [...sigs].reverse()) {
      if (this.ledger.processed.includes(s.signature)) continue;
      this.ledger.processed.push(s.signature);
      if (s.err) continue;
      try {
        const t = await this.parseTransfer(r, s.signature);
        if (t && t.lamports >= this.cfg.minLamports) {
          const bytes = (t.lamports * math.BYTES_PER_GB) / this.cfg.pricePerGbLamports;
          this.ledger.creditedBytes[t.sender] = (this.prepaidBytes(t.sender) + bytes).toString();
        }
      } catch {
        /* skip unparseable tx */
      }
    }
    this.ledger.cursor = newest;
    this.save();
  }
}

function readU64LE(b: Uint8Array, off: number): bigint {
  let v = 0n;
  for (let i = 0; i < 8; i++) v |= BigInt(b[off + i]) << (8n * BigInt(i));
  return v;
}

const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function bs58Decode(s: string): Uint8Array {
  let n = 0n;
  for (const ch of s) {
    const i = B58.indexOf(ch);
    if (i < 0) throw new Error('bad base58');
    n = n * 58n + BigInt(i);
  }
  const bytes: number[] = [];
  while (n > 0n) {
    bytes.unshift(Number(n & 0xffn));
    n >>= 8n;
  }
  for (let i = 0; i < s.length && s[i] === '1'; i++) bytes.unshift(0);
  return Uint8Array.from(bytes);
}
