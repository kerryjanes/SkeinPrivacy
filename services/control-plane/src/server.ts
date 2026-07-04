// Thin HTTP API the clients/website talk to. Three routes:
//   POST /provision  {wallet}              → mint/refresh the user's links + status
//   POST /settle     {wallet, signature}   → register a pay_traffic payment, clear the tab
//   GET  /status?wallet=…                  → current quota / usage / owed
//   GET  /price                            → the public price (1000 WEFT/GB) + node info
// Built on node:http so the only deps are the chain SDK — no web framework.

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { NodeConfig } from './config.js';
import type { Controller } from './controller.js';
import { registerExitProfile } from './exitProfiles.js';
import type { Faucet } from './faucet.js';
import { isRegisteredEndpoint, liveEndpointHashes } from './relay.js';
import { math } from '@weft/sdk';

function send(res: ServerResponse, code: number, body: unknown): void {
  const data = JSON.stringify(body);
  res.writeHead(code, {
    'content-type': 'application/json',
    'access-control-allow-origin': '*',
    // the cabinet POSTs JSON cross-origin → the browser preflights and needs these
    'access-control-allow-methods': 'GET, POST, OPTIONS',
    'access-control-allow-headers': 'content-type',
    'access-control-max-age': '86400',
  });
  res.end(data);
}

function readJson(req: IncomingMessage): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    let buf = '';
    req.on('data', (c) => {
      buf += c;
      if (buf.length > 1 << 16) reject(new Error('body too large'));
    });
    req.on('end', () => {
      try {
        resolve(buf ? (JSON.parse(buf) as Record<string, unknown>) : {});
      } catch {
        reject(new Error('invalid JSON'));
      }
    });
  });
}

function bearer(req: IncomingMessage): string {
  const h = req.headers.authorization ?? '';
  return h.startsWith('Bearer ') ? h.slice('Bearer '.length) : '';
}

export function startServer(cfg: NodeConfig, ctrl: Controller, faucet?: Faucet): void {
  // /relay/live is public and polled by every cabinet; cache the on-chain + frps lookup briefly so
  // it can't be turned into an RPC-exhaustion lever (matches the indexer's 10s directory cache).
  const LIVE_TTL_MS = 10_000;
  let liveCache: { at: number; hashes: string[] } | null = null;

  const server = createServer((req, res) => {
    void handle(req, res).catch((e) => send(res, 400, { error: String(e?.message ?? e) }));
  });

  async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const url = new URL(req.url ?? '/', 'http://localhost');
    if (req.method === 'OPTIONS') return send(res, 204, {});

    if (req.method === 'POST' && url.pathname === '/provision') {
      const { wallet } = await readJson(req);
      if (typeof wallet !== 'string') return send(res, 400, { error: 'wallet required' });
      return send(res, 200, await ctrl.provision(wallet));
    }

    if (req.method === 'POST' && url.pathname === '/settle') {
      const { wallet, signature } = await readJson(req);
      if (typeof wallet !== 'string' || typeof signature !== 'string')
        return send(res, 400, { error: 'wallet + signature required' });
      return send(res, 200, await ctrl.settle(wallet, signature));
    }

    if (req.method === 'GET' && url.pathname === '/status') {
      const wallet = url.searchParams.get('wallet');
      if (!wallet) return send(res, 400, { error: 'wallet required' });
      // /status just reads — provision is idempotent and also refreshes the balance
      return send(res, 200, await ctrl.provision(wallet));
    }

    if (req.method === 'POST' && url.pathname === '/faucet') {
      if (!faucet) return send(res, 404, { error: 'faucet disabled (not a test-mint node)' });
      const { wallet } = await readJson(req);
      if (typeof wallet !== 'string') return send(res, 400, { error: 'wallet required' });
      return send(res, 200, await faucet.drip(wallet));
    }

    if (req.method === 'POST' && url.pathname === '/faucet-sol') {
      if (!faucet) return send(res, 404, { error: 'faucet disabled (not a test-mint node)' });
      const { wallet } = await readJson(req);
      if (typeof wallet !== 'string') return send(res, 400, { error: 'wallet required' });
      return send(res, 200, await faucet.dripSol(wallet));
    }

    if (req.method === 'GET' && url.pathname === '/relay/live') {
      // endpointHashes of nodes carrying traffic right now (for the cabinet's "live nodes" filter)
      const now = Date.now();
      if (!liveCache || now - liveCache.at > LIVE_TTL_MS) {
        liveCache = { at: now, hashes: await liveEndpointHashes(cfg) };
      }
      return send(res, 200, { endpointHashes: liveCache.hashes });
    }

    if (req.method === 'POST' && url.pathname === '/relay/node-profile') {
      if (bearer(req) !== cfg.relayToken) return send(res, 401, { error: 'unauthorized' });
      const body = await readJson(req);
      const host = typeof body.host === 'string' ? body.host : '';
      const port = Number(body.port);
      if (!(await isRegisteredEndpoint(cfg, host, port))) {
        return send(res, 409, { error: 'node endpoint is not registered on-chain' });
      }
      const profile = registerExitProfile(cfg, body);
      ctrl.reconcile(); // fresh profile can become the active hop1 exit immediately
      return send(res, 200, { ok: true, endpoint: `${profile.host}:${profile.port}` });
    }

    if (req.method === 'GET' && url.pathname === '/node/stats') {
      // The operator's earnings basis: total traffic this node has served → $WEFT owed.
      return send(res, 200, { host: cfg.host, ...ctrl.nodeStats() });
    }

    if (req.method === 'GET' && url.pathname === '/price') {
      return send(res, 200, {
        weftPerGb: Number(math.USER_PRICE_PER_GB / math.ONE_WEFT), // whole $WEFT per GB (decimals-independent)
        mint: cfg.weftMint,
        host: cfg.host,
        modes: ['1hop', 'multihop'],
        faucet: !!faucet, // devnet test-$WEFT faucet available?
      });
    }

    return send(res, 404, { error: 'not found' });
  }

  server.listen(cfg.port, () => {
    // eslint-disable-next-line no-console
    console.log(`weft control-plane on :${cfg.port} (node ${cfg.host}, mint ${cfg.weftMint})`);
  });
}
