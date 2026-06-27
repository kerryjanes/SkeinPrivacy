// The node-control surface: render the full xray config from the active user set, reload xray to
// apply it, and read per-user traffic from xray's stats API.
//
// User add/remove is done by re-rendering config.json + `systemctl restart xray`. It's the
// dead-simple, reliable path (no fragile gRPC); the brief reconnect on reload is acceptable at
// launch scale. Metering uses `xray api statsquery -reset`, which returns the delta since the last
// call — so usage keeps accumulating in the store across both polls and xray restarts.

import { execFileSync, execSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import type { NodeConfig } from './config.js';
import { liveExitProfiles, type ExitProfile } from './exitProfiles.js';
import type { User } from './store.js';

interface Client {
  id: string;
  email: string;
  flow?: string;
}

interface Outbound {
  tag: string;
  protocol: string;
  sendThrough?: string;
  settings?: unknown;
  streamSettings?: unknown;
}

function userExitOutbound(profile: ExitProfile): Outbound {
  return {
    tag: `user-exit-${profile.port}`,
    protocol: 'vless',
    settings: {
      vnext: [
        {
          address: profile.host,
          port: profile.port,
          users: [
            {
              id: profile.uuid,
              encryption: 'none',
              flow: 'xtls-rprx-vision',
            },
          ],
        },
      ],
    },
    streamSettings: {
      network: 'tcp',
      security: 'reality',
      realitySettings: {
        serverName: profile.sni,
        fingerprint: 'firefox',
        publicKey: profile.realityPub,
        shortId: profile.sid,
        spiderX: '/',
      },
    },
  };
}

export function renderConfig(cfg: NodeConfig, activeUsers: User[]): unknown {
  const reality = {
    show: false,
    dest: `${cfg.sni}:443`,
    xver: 0,
    serverNames: [cfg.sni],
    privateKey: cfg.realityPrivateKey,
    shortIds: [cfg.shortId],
  };
  // A home node is 1-hop ONLY (hopnPort <= 0): it never runs Tor and never becomes a multihop relay.
  // Multihop is always served over Tor by infrastructure nodes that set a real hopnPort.
  const multihop = cfg.hopnPort > 0;
  // founder stays present + unmetered so the original launch link never breaks
  const hop1Clients: Client[] = [
    { id: cfg.founderUuid, email: 'founder', flow: 'xtls-rprx-vision' },
    ...activeUsers.map((u) => ({ id: u.uuid, email: u.email, flow: 'xtls-rprx-vision' })),
  ];
  const hopnClients: Client[] = [
    { id: cfg.founderUuid, email: 'founder' },
    ...activeUsers.map((u) => ({ id: u.uuid, email: u.email })),
  ];
  const directOutbound: Outbound = {
    tag: 'direct',
    protocol: 'freedom',
    settings: cfg.xraySendThrough ? { domainStrategy: 'UseIPv4' } : undefined,
  };
  if (cfg.xraySendThrough) directOutbound.sendThrough = cfg.xraySendThrough;
  const userExits = multihop ? liveExitProfiles(cfg).map(userExitOutbound) : [];
  const userExitBalancerTag = 'user-exit-balancer';

  return {
    log: { loglevel: 'warning' },
    api: { tag: 'api', services: ['HandlerService', 'StatsService'] },
    stats: {},
    policy: {
      levels: { '0': { statsUserUplink: true, statsUserDownlink: true } },
      system: { statsInboundUplink: true, statsInboundDownlink: true },
    },
    inbounds: [
      {
        tag: 'api',
        listen: '127.0.0.1',
        port: Number(cfg.xrayApi.split(':')[1] ?? 10085),
        protocol: 'dokodemo-door',
        settings: { address: '127.0.0.1' },
      },
      {
        tag: 'hop1',
        listen: '0.0.0.0',
        port: cfg.hop1Port,
        protocol: 'vless',
        settings: { clients: hop1Clients, decryption: 'none' },
        streamSettings: { network: 'tcp', security: 'reality', realitySettings: reality },
      },
      ...(multihop
        ? [
            {
              tag: 'hopN',
              listen: '0.0.0.0',
              port: cfg.hopnPort,
              protocol: 'vless',
              settings: { clients: hopnClients, decryption: 'none' },
              streamSettings: { network: 'tcp', security: 'reality', realitySettings: reality },
            },
          ]
        : []),
    ],
    outbounds: [
      directOutbound,
      ...userExits,
      ...(multihop
        ? [
            {
              tag: 'block',
              protocol: 'blackhole',
            },
            {
              tag: 'tor',
              protocol: 'socks',
              settings: { servers: [{ address: '127.0.0.1', port: 9050 }] },
            },
          ]
        : []),
    ],
    routing: {
      balancers: userExits.length
        ? [
            {
              tag: userExitBalancerTag,
              selector: ['user-exit-'],
              fallbackTag: 'direct',
              strategy: { type: 'roundRobin' },
            },
          ]
        : [],
      rules: [
        { type: 'field', inboundTag: ['api'], outboundTag: 'api' },
        userExits.length
          ? { type: 'field', inboundTag: ['hop1'], balancerTag: userExitBalancerTag }
          : { type: 'field', inboundTag: ['hop1'], outboundTag: 'direct' },
        ...(multihop
          ? [
              // Tor is TCP-only. Reject UDP/QUIC quickly so mobile clients fall back to TCP instead
              // of waiting for UDP timeouts on connectivity probes.
              { type: 'field', inboundTag: ['hopN'], network: 'udp', outboundTag: 'block' },
              { type: 'field', inboundTag: ['hopN'], network: 'tcp', outboundTag: 'tor' },
            ]
          : []),
      ],
    },
  };
}

/** Write the rendered config and reload xray so it takes effect. */
export function applyConfig(cfg: NodeConfig, activeUsers: User[]): void {
  writeFileSync(cfg.xrayConfigPath, JSON.stringify(renderConfig(cfg, activeUsers), null, 2));
  execSync(cfg.reloadCmd, { stdio: 'ignore' });
}

/**
 * Per-user traffic since the last call (uplink+downlink), keyed by email. `-reset` zeroes the
 * counters so each poll yields a delta we add to the store.
 */
export function pollUsage(cfg: NodeConfig): Map<string, bigint> {
  let raw: string;
  try {
    raw = execFileSync(
      cfg.xrayBin,
      ['api', 'statsquery', `--server=${cfg.xrayApi}`, '-pattern', 'user>>>', '-reset'],
      { encoding: 'utf8' },
    );
  } catch {
    return new Map(); // api unreachable this tick → no delta (counters keep accruing for next time)
  }
  return parseUsage(raw);
}

/** Parse `xray api statsquery` JSON into uplink+downlink totals keyed by user email. */
export function parseUsage(raw: string): Map<string, bigint> {
  const out = new Map<string, bigint>();
  const parsed = JSON.parse(raw || '{}') as { stat?: { name: string; value?: string }[] };
  for (const s of parsed.stat ?? []) {
    // name = user>>>EMAIL>>>traffic>>>uplink|downlink
    const email = s.name.split('>>>')[1];
    if (!email) continue;
    out.set(email, (out.get(email) ?? 0n) + BigInt(s.value ?? '0'));
  }
  return out;
}
