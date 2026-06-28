// In-memory epoch/proof store. Holds the built epochs the aggregator has posted
// so `GET /proof?epoch&operator&nodeId` can hand a claimant exactly the proof
// the on-chain `claim` will accept. (Production would persist this; the shape is
// deliberately serializable.)

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import type { EpochBuild, EpochEntry } from './rewards';

export interface ClaimableEntry extends EpochEntry {
  epoch: bigint;
}

export interface ClaimableNode {
  nodeId: bigint;
  totalAmount: bigint;
  claims: ClaimableEntry[];
}

export interface ClaimableSummary {
  operator: string;
  totalAmount: bigint;
  nodes: ClaimableNode[];
}

export class EpochStore {
  private byEpoch = new Map<string, EpochBuild>();

  constructor(private path = '') {
    if (!path || !existsSync(path)) return;
    const rows = JSON.parse(readFileSync(path, 'utf8'), (_key, value) => {
      if (value && typeof value === 'object' && typeof value.__bigint === 'string') {
        return BigInt(value.__bigint);
      }
      return value;
    }) as EpochBuild[];
    for (const row of rows) this.byEpoch.set(row.epoch.toString(), row);
  }

  put(build: EpochBuild): void {
    this.byEpoch.set(build.epoch.toString(), build);
    this.save();
  }

  get(epoch: bigint): EpochBuild | undefined {
    return this.byEpoch.get(epoch.toString());
  }

  proof(epoch: bigint, operator: string, nodeId: bigint): EpochEntry | undefined {
    const build = this.get(epoch);
    if (!build) return undefined;
    return build.entries.find((e) => e.operator === operator && e.nodeId === nodeId);
  }

  epochs(): bigint[] {
    return [...this.byEpoch.values()].map((b) => b.epoch);
  }

  maxEpoch(): bigint | null {
    let max: bigint | null = null;
    for (const build of this.byEpoch.values()) {
      if (max === null || build.epoch > max) max = build.epoch;
    }
    return max;
  }

  claimable(operator: string): ClaimableSummary {
    const byNode = new Map<string, ClaimableNode>();
    for (const build of this.byEpoch.values()) {
      for (const entry of build.entries) {
        if (entry.operator !== operator) continue;
        const key = entry.nodeId.toString();
        const node = byNode.get(key) ?? { nodeId: entry.nodeId, totalAmount: 0n, claims: [] };
        node.totalAmount += entry.amount;
        node.claims.push({ ...entry, epoch: build.epoch });
        byNode.set(key, node);
      }
    }
    const nodes = [...byNode.values()]
      .map((node) => ({
        ...node,
        claims: node.claims.sort((a, b) => (a.epoch > b.epoch ? -1 : a.epoch < b.epoch ? 1 : 0)),
      }))
      .sort((a, b) => (a.nodeId < b.nodeId ? -1 : a.nodeId > b.nodeId ? 1 : 0));
    return {
      operator,
      totalAmount: nodes.reduce((sum, node) => sum + node.totalAmount, 0n),
      nodes,
    };
  }

  private save(): void {
    if (!this.path) return;
    const dir = dirname(this.path);
    if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
    const tmp = `${this.path}.tmp`;
    writeFileSync(
      tmp,
      JSON.stringify(
        [...this.byEpoch.values()],
        (_key, value) => (typeof value === 'bigint' ? { __bigint: value.toString() } : value),
        2,
      ),
    );
    renameSync(tmp, this.path);
  }
}

export interface PayoutRecord {
  operator: string;
  nodeId: string;
  amount: string;
  signature: string;
  createdAt: number;
}

export interface PayoutData {
  paid: Record<string, string>;
  records: PayoutRecord[];
}

function payoutKey(operator: string, nodeId: bigint): string {
  return `${operator}:${nodeId}`;
}

export class PayoutStore {
  private data: PayoutData;

  constructor(private path = '') {
    this.data =
      path && existsSync(path)
        ? (JSON.parse(readFileSync(path, 'utf8')) as PayoutData)
        : { paid: {}, records: [] };
    if (!this.data.paid) this.data.paid = {};
    if (!this.data.records) this.data.records = [];
  }

  paid(operator: string, nodeId: bigint): bigint {
    return BigInt(this.data.paid[payoutKey(operator, nodeId)] ?? '0');
  }

  record(operator: string, nodeId: bigint, amount: bigint, signature: string, now = Date.now()): void {
    if (amount <= 0n) return;
    const key = payoutKey(operator, nodeId);
    this.data.paid[key] = (BigInt(this.data.paid[key] ?? '0') + amount).toString();
    this.data.records.push({
      operator,
      nodeId: nodeId.toString(),
      amount: amount.toString(),
      signature,
      createdAt: now,
    });
    this.save();
  }

  private save(): void {
    if (!this.path) return;
    const dir = dirname(this.path);
    if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
    const tmp = `${this.path}.tmp`;
    writeFileSync(tmp, JSON.stringify(this.data, null, 2));
    renameSync(tmp, this.path);
  }
}
