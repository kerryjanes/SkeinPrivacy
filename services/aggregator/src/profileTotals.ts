import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import type { Address } from '@solana/kit';

import type { ByteTotal, NodeInfo } from './rewards';

export interface RelayProfileStats {
  host: string;
  port: number;
  servedBytesLifetime?: string;
}

export type SettledProfileBytes = Record<string, string>;

function endpoint(profile: Pick<RelayProfileStats, 'host' | 'port'>): string {
  return `${profile.host}:${profile.port}`;
}

export function endpointHashHex(endpointValue: string): string {
  return createHash('sha256').update(endpointValue).digest('hex');
}

export function readSettledProfileBytes(path: string): SettledProfileBytes {
  if (!existsSync(path)) return {};
  return JSON.parse(readFileSync(path, 'utf8')) as SettledProfileBytes;
}

export function writeSettledProfileBytes(path: string, data: SettledProfileBytes): void {
  const dir = dirname(path);
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
  const tmp = `${path}.tmp`;
  writeFileSync(tmp, JSON.stringify(data, null, 2));
  renameSync(tmp, path);
}

export function readRelayProfiles(path: string): RelayProfileStats[] {
  if (!existsSync(path)) return [];
  return Object.values(JSON.parse(readFileSync(path, 'utf8')) as Record<string, RelayProfileStats>);
}

export function buildProfileByteTotals(
  profiles: RelayProfileStats[],
  nodes: Array<NodeInfo & { endpointHash?: string }>,
  settled: SettledProfileBytes,
): { totals: ByteTotal[]; nextSettled: SettledProfileBytes } {
  const nodeByEndpointHash = new Map<string, NodeInfo>();
  for (const node of nodes) {
    if (node.endpointHash) nodeByEndpointHash.set(node.endpointHash, node);
  }

  const totals: ByteTotal[] = [];
  const nextSettled: SettledProfileBytes = { ...settled };
  for (const profile of profiles) {
    const ep = endpoint(profile);
    const node = nodeByEndpointHash.get(endpointHashHex(ep));
    if (!node) continue;
    const lifetime = BigInt(profile.servedBytesLifetime ?? '0');
    const previous = BigInt(settled[ep] ?? '0');
    if (lifetime <= previous) continue;
    totals.push({
      operator: node.operator as Address,
      nodeId: node.nodeId,
      bytes: lifetime - previous,
    });
    nextSettled[ep] = lifetime.toString();
  }
  return { totals, nextSettled };
}
