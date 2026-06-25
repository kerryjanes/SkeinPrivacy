// Thin HTTP API the clients/website talk to. Three routes:
//   POST /provision  {wallet}              → mint/refresh the user's links + status
//   POST /settle     {wallet, signature}   → register a pay_traffic payment, clear the tab
//   GET  /status?wallet=…                  → current quota / usage / owed
//   GET  /price                            → the public price (0.1 WEFT/GB) + node info
// Built on node:http so the only deps are the chain SDK — no web framework.

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { NodeConfig } from './config.js';
import type { Controller } from './controller.js';
import { math } from '@weft/sdk';

function send(res: ServerResponse, code: number, body: unknown): void {
  const data = JSON.stringify(body);
  res.writeHead(code, { 'content-type': 'application/json', 'access-control-allow-origin': '*' });
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

export function startServer(cfg: NodeConfig, ctrl: Controller): void {
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

    if (req.method === 'GET' && url.pathname === '/price') {
      return send(res, 200, {
        weftPerGb: Number(math.BASE_RATE_PER_GB) / 1e9,
        mint: cfg.weftMint,
        host: cfg.host,
        modes: ['1hop', 'multihop'],
      });
    }

    return send(res, 404, { error: 'not found' });
  }

  server.listen(cfg.port, () => {
    // eslint-disable-next-line no-console
    console.log(`weft control-plane on :${cfg.port} (node ${cfg.host}, mint ${cfg.weftMint})`);
  });
}
