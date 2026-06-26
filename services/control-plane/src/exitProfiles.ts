import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import type { NodeConfig } from './config.js';

export interface ExitProfile {
  host: string;
  port: number;
  uuid: string;
  realityPub: string;
  sid: string;
  sni: string;
  geo: number;
  servedBytesLifetime: string;
  updatedAt: number;
}

type ExitProfileInput = Omit<ExitProfile, 'updatedAt'>;

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const SID_RE = /^[0-9a-f]{2,32}$/i;

function key(profile: Pick<ExitProfile, 'host' | 'port'>): string {
  return `${profile.host}:${profile.port}`;
}

function readStore(path: string): Record<string, ExitProfile> {
  if (!existsSync(path)) return {};
  return JSON.parse(readFileSync(path, 'utf8')) as Record<string, ExitProfile>;
}

function writeStore(path: string, profiles: Record<string, ExitProfile>): void {
  const dir = dirname(path);
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
  const tmp = `${path}.tmp`;
  writeFileSync(tmp, JSON.stringify(profiles, null, 2));
  renameSync(tmp, path);
}

export function registerExitProfile(cfg: NodeConfig, input: unknown, now = Date.now()): ExitProfile {
  const p = input as Partial<ExitProfileInput>;
  if (p.host !== cfg.host) throw new Error('profile host must match relay host');
  const port = p.port;
  if (!Number.isInteger(port) || port === undefined || port < 20000 || port > 20079) {
    throw new Error('profile port must be in the relay user-node range');
  }
  if (typeof p.uuid !== 'string' || !UUID_RE.test(p.uuid)) throw new Error('invalid profile uuid');
  if (typeof p.realityPub !== 'string' || p.realityPub.length < 20) {
    throw new Error('invalid profile reality public key');
  }
  if (typeof p.sid !== 'string' || !SID_RE.test(p.sid)) throw new Error('invalid profile sid');
  if (typeof p.sni !== 'string' || p.sni.length < 3) throw new Error('invalid profile sni');
  const profile: ExitProfile = {
    host: p.host,
    port,
    uuid: p.uuid,
    realityPub: p.realityPub,
    sid: p.sid,
    sni: p.sni,
    geo: Number(p.geo ?? 0),
    servedBytesLifetime: BigInt(String(p.servedBytesLifetime ?? 0)).toString(),
    updatedAt: now,
  };
  const profiles = readStore(cfg.relayProfilePath);
  profiles[key(profile)] = profile;
  writeStore(cfg.relayProfilePath, profiles);
  return profile;
}

export function liveExitProfiles(cfg: NodeConfig, now = Date.now()): ExitProfile[] {
  const profiles = Object.values(readStore(cfg.relayProfilePath));
  return profiles
    .filter((p) => now - p.updatedAt <= cfg.exitProfileTtlMs)
    .sort((a, b) => `${a.host}:${a.port}`.localeCompare(`${b.host}:${b.port}`));
}

export function exitProfileSignature(cfg: NodeConfig, now = Date.now()): string {
  return liveExitProfiles(cfg, now)
    .map((p) => `${p.host}:${p.port}:${p.uuid}:${p.realityPub}:${p.sid}`)
    .join(',');
}

export async function publishOwnExitProfile(cfg: NodeConfig, servedBytesLifetime = 0n): Promise<void> {
  if (!cfg.relayProfileUrl) return;
  const body: ExitProfileInput = {
    host: cfg.host,
    port: cfg.publicHop1Port,
    uuid: cfg.founderUuid,
    realityPub: cfg.realityPublicKey,
    sid: cfg.shortId,
    sni: cfg.sni,
    geo: cfg.geo,
    servedBytesLifetime: servedBytesLifetime.toString(),
  };
  const res = await fetch(cfg.relayProfileUrl, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${cfg.relayToken}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`relay profile publish failed: HTTP ${res.status}`);
}
