// Aggregator HTTP surface: serve claim proofs and the Solana Pay endpoints.
//   GET  /health
//   GET  /epoch?epoch=N                  → built-epoch summary
//   GET  /proof?epoch=N&operator=&nodeId= → the exact leaf + proof `claim` accepts
//   GET  /pay/traffic                    → Solana Pay label (wallet handshake)
//   POST /pay/traffic?amount=N {account} → unsigned pay_traffic transaction
//   POST /receipts?epoch=N {receipts}    → ingest dual-signed traffic receipts (M6)

import { createServer, type Server } from 'node:http';
import { address, type Address } from '@solana/kit';

import { buildPayTrafficTransaction, payLabel, type Blockhashish, type PayConfig } from './pay';
import { selectReceiptsForEpoch, type TrafficReceipt } from './receipts';
import type { EpochStore } from './store';

export interface ServerDeps {
  store: EpochStore;
  payConfig: PayConfig;
  getBlockhash: () => Promise<Blockhashish>;
  /** Called with the verified, deduped receipts ingested for an epoch (M6 → M4). */
  onReceipts?: (epoch: bigint, accepted: TrafficReceipt[]) => Promise<unknown> | unknown;
}

function readBody(req: import('node:http').IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    let data = '';
    req.on('data', (c) => (data += c));
    req.on('end', () => resolve(data));
    req.on('error', reject);
  });
}

function parseReceipt(raw: unknown): TrafficReceipt {
  const r = raw as Record<string, unknown>;
  return {
    client: address(String(r.client)) as Address,
    operator: address(String(r.operator)) as Address,
    nodeId: BigInt(String(r.nodeId)),
    bytes: BigInt(String(r.bytes)),
    windowStart: BigInt(String(r.windowStart)),
    windowEnd: BigInt(String(r.windowEnd)),
    nonce: BigInt(String(r.nonce)),
    clientSig: String(r.clientSig),
    relaySig: String(r.relaySig),
  };
}

export function createAggregatorServer(deps: ServerDeps): Server {
  return createServer((req, res) => {
    void (async () => {
      try {
        const url = new URL(req.url ?? '/', 'http://localhost');
        const json = (code: number, body: unknown) => {
          res.writeHead(code, { 'content-type': 'application/json' });
          res.end(JSON.stringify(body));
        };

        if (url.pathname === '/health') {
          json(200, { ok: true, epochs: deps.store.epochs().map(String) });
          return;
        }

        if (url.pathname === '/epoch' && req.method === 'GET') {
          const epoch = BigInt(url.searchParams.get('epoch') ?? '-1');
          const build = deps.store.get(epoch);
          if (!build) {
            json(404, { error: 'epoch not built' });
            return;
          }
          json(200, {
            epoch: build.epoch.toString(),
            root: build.root,
            totalReward: build.totalReward.toString(),
            numNodes: build.numNodes,
          });
          return;
        }

        if (url.pathname === '/proof' && req.method === 'GET') {
          const epoch = BigInt(url.searchParams.get('epoch') ?? '-1');
          const operator = url.searchParams.get('operator') ?? '';
          const nodeId = BigInt(url.searchParams.get('nodeId') ?? '-1');
          const entry = deps.store.proof(epoch, operator, nodeId);
          if (!entry) {
            json(404, { error: 'no proof for (epoch, operator, nodeId)' });
            return;
          }
          json(200, {
            epoch: epoch.toString(),
            operator: entry.operator,
            nodeId: entry.nodeId.toString(),
            amount: entry.amount.toString(),
            leaf: entry.leaf,
            proof: entry.proof,
          });
          return;
        }

        if (url.pathname === '/pay/traffic' && req.method === 'GET') {
          json(200, payLabel(deps.payConfig));
          return;
        }

        if (url.pathname === '/pay/traffic' && req.method === 'POST') {
          const amount = BigInt(url.searchParams.get('amount') ?? '0');
          if (amount <= 0n) {
            json(400, { error: 'amount must be positive' });
            return;
          }
          const body = JSON.parse((await readBody(req)) || '{}') as { account?: string };
          if (!body.account) {
            json(400, { error: 'missing account' });
            return;
          }
          const account = address(body.account) as Address;
          const tx = await buildPayTrafficTransaction(
            account,
            amount,
            deps.payConfig,
            await deps.getBlockhash(),
          );
          json(200, tx);
          return;
        }

        if (url.pathname === '/receipts' && req.method === 'POST') {
          const epoch = BigInt(url.searchParams.get('epoch') ?? '-1');
          if (epoch < 0n) {
            json(400, { error: 'missing epoch' });
            return;
          }
          const body = JSON.parse((await readBody(req)) || '{}') as { receipts?: unknown };
          const raw = Array.isArray(body.receipts) ? body.receipts.map(parseReceipt) : [];
          // verify both signatures, drop out-of-epoch + duplicate (operator, nonce).
          const sel = selectReceiptsForEpoch(raw, epoch);
          const result = await deps.onReceipts?.(epoch, sel.accepted);
          json(200, {
            epoch: epoch.toString(),
            accepted: sel.accepted.length,
            rejected: sel.rejected.length,
            reasons: sel.rejected.map((r) => r.reason),
            result,
          });
          return;
        }

        json(404, { error: 'not found' });
      } catch (e) {
        res.writeHead(500, { 'content-type': 'application/json' });
        res.end(JSON.stringify({ error: String(e) }));
      }
    })();
  });
}
