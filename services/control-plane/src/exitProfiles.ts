import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
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
  lastReportedServedBytes?: string;
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
  const profiles = readStore(cfg.relayProfilePath);
  const incomingServed = BigInt(String(p.servedBytesLifetime ?? 0));
  const existing = profiles[key({ host: p.host, port })];
  const existingServed = BigInt(existing?.servedBytesLifetime ?? '0');
  const lastReported = BigInt(existing?.lastReportedServedBytes ?? existing?.servedBytesLifetime ?? '0');
  const servedBytesLifetime =
    existing && incomingServed < lastReported
      ? existingServed + incomingServed
      : existingServed + (incomingServed > lastReported ? incomingServed - lastReported : 0n);
  const profile: ExitProfile = {
    host: p.host,
    port,
    uuid: p.uuid,
    realityPub: p.realityPub,
    sid: p.sid,
    sni: p.sni,
    geo: Number(p.geo ?? 0),
    servedBytesLifetime: servedBytesLifetime.toString(),
    lastReportedServedBytes: incomingServed.toString(),
    updatedAt: now,
  };
  profiles[key(profile)] = profile;
  writeStore(cfg.relayProfilePath, profiles);
  return profile;
}

export function liveExitProfiles(cfg: NodeConfig, now = Date.now()): ExitProfile[] {
  const onlinePorts = relayOnlinePorts(cfg);
  return liveExitProfilesWithPorts(cfg, now, onlinePorts);
}

export function liveExitProfilesWithPorts(
  cfg: NodeConfig,
  now = Date.now(),
  onlinePorts: Set<number> | null = null,
): ExitProfile[] {
  const profiles = Object.values(readStore(cfg.relayProfilePath));
  return profiles
    .filter((p) => {
      if (onlinePorts) return onlinePorts.has(p.port);
      const fresh = now - p.updatedAt <= cfg.exitProfileTtlMs;
      return fresh;
    })
    .sort((a, b) => `${a.host}:${a.port}`.localeCompare(`${b.host}:${b.port}`));
}

export function exitProfileSignature(cfg: NodeConfig, now = Date.now()): string {
  return liveExitProfiles(cfg, now)
    .map((p) => `${p.host}:${p.port}:${p.uuid}:${p.realityPub}:${p.sid}`)
    .join(',');
}

function relayOnlinePorts(cfg: NodeConfig): Set<number> | null {
  if (!cfg.frpsApi) return null;
  try {
    const auth = Buffer.from(`${cfg.frpsUser}:${cfg.frpsPass}`).toString('base64');
    const raw = execFileSync(
      'curl',
      ['-fsS', '--max-time', '2', '-H', `Authorization: Basic ${auth}`, `${cfg.frpsApi}/api/proxy/tcp`],
      { encoding: 'utf8' },
    );
    const j = JSON.parse(raw) as { proxies?: { status?: string; conf?: { remotePort?: number } }[] };
    return new Set(
      (j.proxies ?? [])
        .filter((p) => p.status === 'online' && Number.isInteger(p.conf?.remotePort))
        .map((p) => Number(p.conf?.remotePort)),
    );
  } catch {
    return null;
  }
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
