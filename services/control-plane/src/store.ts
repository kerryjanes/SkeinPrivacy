// Durable per-user state. One JSON file; small enough to rewrite atomically on every change.
// Survives restarts (and xray's stats counters resetting) because usage is accumulated here.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

export interface User {
  wallet: string; // Solana pubkey (base58) — the identity + the $WEFT payer
  uuid: string; // VLESS id baked into this user's personal link
  email: string; // xray stats key (`user>>>email>>>traffic>>>…`)
  unsettledBytes: string; // metered consumption not yet paid for (bigint as string)
  servedBytesLifetime: string; // cumulative bytes this node served this user — never reset; the
  // basis for the operator's $WEFT earnings (settlement clears the tab, but earnings keep counting)
  balanceBaseUnits: string; // last observed prepaid escrow balance
  quotaBytes: string; // bytes that prepaid balance buys
  active: boolean; // currently present in the xray config (link works)
  createdAt: number;
}

export interface PaymentRecord {
  wallet: string;
  signature: string;
  amountBaseUnits: string;
  status: 'pending' | 'processed';
  createdAt: number;
  processedAt?: number;
}

export interface StoreData {
  users: Record<string, User>; // keyed by wallet
  payments: Record<string, PaymentRecord>; // keyed by settlement tx signature
  nodeServedBytesLifetime?: string; // raw cumulative traffic this node carried, including founder/relay users
}

export class Store {
  private data: StoreData;
  constructor(private path: string) {
    let loaded: StoreData | null = null;
    if (existsSync(path)) {
      try {
        loaded = JSON.parse(readFileSync(path, 'utf8')) as StoreData;
      } catch (e) {
        // A corrupt store must not crash-loop the node: quarantine it and start clean. Users
        // re-provision (quota re-derives from on-chain escrow); only in-flight unsettled tabs reset.
        const quarantine = `${path}.corrupt-${process.pid}`;
        try {
          renameSync(path, quarantine);
        } catch {
          /* best effort */
        }
        console.error(
          `[control-plane] user store unreadable (${(e as Error).message}); quarantined to ${quarantine}`,
        );
      }
    }
    this.data = loaded ?? { users: {}, payments: {} };
    if (!this.data.users) this.data.users = {};
    if (!this.data.payments) this.data.payments = {};
    if (this.data.nodeServedBytesLifetime === undefined) this.data.nodeServedBytesLifetime = '0';
  }

  get(wallet: string): User | undefined {
    return this.data.users[wallet];
  }

  all(): User[] {
    return Object.values(this.data.users);
  }

  nodeServedBytesLifetime(): string {
    return this.data.nodeServedBytesLifetime ?? '0';
  }

  addNodeServedBytes(bytes: bigint): void {
    if (bytes <= 0n) return;
    this.data.nodeServedBytesLifetime = (BigInt(this.nodeServedBytesLifetime()) + bytes).toString();
    this.save();
  }

  put(user: User): void {
    this.data.users[user.wallet] = user;
    this.save();
  }

  /** Persist several users in a single file write. The metering loop mutates every user each
   *  tick; writing once (not per user) keeps that O(N), not O(N²). */
  putMany(users: User[]): void {
    for (const u of users) this.data.users[u.wallet] = u;
    this.save();
  }

  payment(signature: string): PaymentRecord | undefined {
    return this.data.payments[signature];
  }

  beginPayment(signature: string, wallet: string, now = Date.now()): void {
    const existing = this.payment(signature);
    if (existing) {
      if (existing.wallet !== wallet) throw new Error('payment signature already submitted');
      if (existing.status === 'processed') throw new Error('payment already processed');
      return; // pending for this wallet (e.g. a prior crash) — re-driving is safe, verify is idempotent
    }
    this.data.payments[signature] = {
      wallet,
      signature,
      amountBaseUnits: '0',
      status: 'pending',
      createdAt: now,
    };
    this.save();
  }

  /**
   * Atomically clear a settled tab and mark its signature processed in ONE write. Because the tab
   * reduction and the "processed" flip land together, a crash can never leave a tab reduced with the
   * signature still pending (a re-drive would double-reduce it) nor a payment stuck pending after the
   * tab was cleared (the user paying on-chain for nothing). No-ops (returns false) if the signature
   * is not a pending reservation for this wallet — so a concurrent double-submit applies exactly once.
   */
  applySettlement(user: User, signature: string, amountBaseUnits: bigint, now = Date.now()): boolean {
    const existing = this.payment(signature);
    if (!existing || existing.wallet !== user.wallet || existing.status !== 'pending') return false;
    this.data.users[user.wallet] = user;
    this.data.payments[signature] = {
      ...existing,
      amountBaseUnits: amountBaseUnits.toString(),
      status: 'processed',
      processedAt: now,
    };
    this.save();
    return true;
  }

  forgetPendingPayment(signature: string, wallet: string): void {
    const existing = this.payment(signature);
    if (existing?.wallet === wallet && existing.status === 'pending') {
      delete this.data.payments[signature];
      this.save();
    }
  }

  /** Boot-time hygiene: drop pending reservations that never completed and are older than maxAgeMs
   *  (an abandoned settle attempt). Processed records are kept forever as the dedup ledger. */
  sweepStalePendings(maxAgeMs: number, now = Date.now()): void {
    let changed = false;
    for (const [sig, rec] of Object.entries(this.data.payments)) {
      if (rec.status === 'pending' && now - rec.createdAt > maxAgeMs) {
        delete this.data.payments[sig];
        changed = true;
      }
    }
    if (changed) this.save();
  }

  save(): void {
    const dir = dirname(this.path);
    if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
    const tmp = `${this.path}.tmp`;
    writeFileSync(tmp, JSON.stringify(this.data, null, 2));
    renameSync(tmp, this.path); // atomic swap
  }
}
