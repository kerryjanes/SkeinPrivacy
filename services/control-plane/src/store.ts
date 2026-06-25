// Durable per-user state. One JSON file; small enough to rewrite atomically on every change.
// Survives restarts (and xray's stats counters resetting) because usage is accumulated here.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

export interface User {
  wallet: string; // Solana pubkey (base58) — the identity + the $WEFT payer
  uuid: string; // VLESS id baked into this user's personal link
  email: string; // xray stats key (`user>>>email>>>traffic>>>…`)
  unsettledBytes: string; // metered consumption not yet paid for (bigint as string)
  balanceBaseUnits: string; // last observed $WEFT balance
  quotaBytes: string; // bytes that balance buys
  active: boolean; // currently present in the xray config (link works)
  createdAt: number;
}

export interface StoreData {
  users: Record<string, User>; // keyed by wallet
}

export class Store {
  private data: StoreData;
  constructor(private path: string) {
    this.data = existsSync(path)
      ? (JSON.parse(readFileSync(path, 'utf8')) as StoreData)
      : { users: {} };
    if (!this.data.users) this.data.users = {};
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

  save(): void {
    const dir = dirname(this.path);
    if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
    const tmp = `${this.path}.tmp`;
    writeFileSync(tmp, JSON.stringify(this.data, null, 2));
    renameSync(tmp, this.path); // atomic swap
  }
}
