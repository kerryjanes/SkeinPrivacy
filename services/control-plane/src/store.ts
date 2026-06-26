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
}

export class Store {
  private data: StoreData;
  constructor(private path: string) {
    this.data = existsSync(path)
      ? (JSON.parse(readFileSync(path, 'utf8')) as StoreData)
      : { users: {}, payments: {} };
    if (!this.data.users) this.data.users = {};
    if (!this.data.payments) this.data.payments = {};
  }

  get(wallet: string): User | undefined {
    return this.data.users[wallet];
  }

  all(): User[] {
    return Object.values(this.data.users);
  }

  put(user: User): void {
    this.data.users[user.wallet] = user;
    this.save();
  }

  payment(signature: string): PaymentRecord | undefined {
    return this.data.payments[signature];
  }

  beginPayment(signature: string, wallet: string, now = Date.now()): void {
    const existing = this.payment(signature);
    if (existing) {
      throw new Error('payment signature already submitted');
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

  completePayment(signature: string, wallet: string, amountBaseUnits: bigint, now = Date.now()): void {
    const existing = this.payment(signature);
    if (!existing || existing.wallet !== wallet || existing.status !== 'pending') {
      throw new Error('payment signature was not reserved by this wallet');
    }
    this.data.payments[signature] = {
      ...existing,
      amountBaseUnits: amountBaseUnits.toString(),
      status: 'processed',
      processedAt: now,
    };
    this.save();
  }

  forgetPendingPayment(signature: string, wallet: string): void {
    const existing = this.payment(signature);
    if (existing?.wallet === wallet && existing.status === 'pending') {
      delete this.data.payments[signature];
      this.save();
    }
  }

  save(): void {
    const dir = dirname(this.path);
    if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
    const tmp = `${this.path}.tmp`;
    writeFileSync(tmp, JSON.stringify(this.data, null, 2));
    renameSync(tmp, this.path); // atomic swap
  }
}
