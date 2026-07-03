// Relay liveness — which node endpoints are actually carrying traffic RIGHT NOW. A node is live if
// it's directly served by this public box OR it has an online frp tunnel (a home node). The cabinet
// uses this to show only running nodes: stop your node → its tunnel drops → it leaves the list (no
// on-chain change — the registration stays, so restarting never re-pays).

import { createHash } from 'node:crypto';
import { createSolanaRpc, getBase58Decoder, type Base58EncodedBytes } from '@solana/kit';
import { weft } from '@weft/sdk';
import type { NodeConfig } from './config.js';
import { liveExitProfilesWithPorts } from './exitProfiles.js';

const hashEndpoint = (ep: string): string => createHash('sha256').update(ep).digest('hex');
const toHex = (u: ArrayLike<number>): string =>
  Array.from(u, (b) => b.toString(16).padStart(2, '0')).join('');

export function filterRegisteredEndpointHashes(
  hashes: string[],
  registered: Set<string>,
): string[] {
  return hashes.filter((hash) => registered.has(hash));
}

export async function registeredEndpointHashes(cfg: NodeConfig): Promise<Set<string>> {
  const client = createSolanaRpc(cfg.rpcUrl);
  const disc = getBase58Decoder().decode(weft.NODE_STATE_DISCRIMINATOR);
  const accounts = await client
    .getProgramAccounts(weft.WEFT_PROGRAM_ADDRESS, {
      encoding: 'base64',
      filters: [{ memcmp: { offset: 0n, bytes: disc as Base58EncodedBytes, encoding: 'base58' } }],
    })
    .send();
  const decoder = weft.getNodeStateDecoder();
  return new Set(
    accounts.map(({ account }) => {
      let bytes = Buffer.from((account.data as readonly [string, string])[0], 'base64');
      if (bytes.length < decoder.fixedSize) {
        const padded = Buffer.alloc(decoder.fixedSize);
        bytes.copy(padded);
        bytes = padded;
      }
      return toHex(decoder.decode(bytes).endpointHash);
    }),
  );
}

/** sha256(host:port) for every endpoint live right now — matches each node's on-chain endpointHash. */
export async function liveEndpointHashes(cfg: NodeConfig): Promise<string[]> {
  const endpoints = new Set<string>();
  // this public box's own directly-served endpoints (e.g. the launch node on :443/:8443)
  for (const p of [cfg.publicHop1Port, cfg.publicHopnPort, cfg.hop1Port, cfg.hopnPort])
    endpoints.add(`${cfg.host}:${p}`);
  // online frp tunnels (home nodes behind NAT), from the frps admin API
  if (cfg.frpsApi) {
    try {
      const auth = Buffer.from(`${cfg.frpsUser}:${cfg.frpsPass}`).toString('base64');
      const r = await fetch(`${cfg.frpsApi}/api/proxy/tcp`, {
        headers: { authorization: `Basic ${auth}` },
      });
      const j = (await r.json()) as {
        proxies?: { status?: string; conf?: { remotePort?: number } }[];
      };
      const onlinePorts = new Set(
        (j.proxies ?? [])
          .filter((p) => p.status === 'online' && Number.isInteger(p.conf?.remotePort))
          .map((p) => Number(p.conf?.remotePort)),
      );
      for (const p of liveExitProfilesWithPorts(cfg, Date.now(), onlinePorts))
        endpoints.add(`${cfg.host}:${p.port}`);
    } catch {
      /* relay API unreachable → just the direct endpoints */
    }
  }
  const liveHashes = [...endpoints].map(hashEndpoint);
  return filterRegisteredEndpointHashes(liveHashes, await registeredEndpointHashes(cfg));
}

export async function isRegisteredEndpoint(
  cfg: NodeConfig,
  host: string,
  port: number,
): Promise<boolean> {
  return (await registeredEndpointHashes(cfg)).has(hashEndpoint(`${host}:${port}`));
}
