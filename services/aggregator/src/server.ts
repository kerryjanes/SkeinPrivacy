// Aggregator HTTP surface: serve claim proofs and the Solana Pay endpoints.
//   GET  /health
//   GET  /epoch?epoch=N                  → built-epoch summary
//   GET  /proof?epoch=N&operator=&nodeId= → the exact leaf + proof `claim` accepts
//   GET  /pay/traffic                    → Solana Pay label (wallet handshake)
//   POST /pay/escrow/deposit?amount=N {account} → unsigned deposit_escrow transaction
//   POST /pay/traffic/escrow?amount=N {account} → unsigned pay_traffic_from_escrow transaction
//   POST /pay/traffic?amount=N {account} → legacy unsigned pay_traffic transaction
//   POST /receipts?epoch=N {receipts}    → ingest dual-signed traffic receipts (M6)

import { createServer, type Server } from 'node:http';
import { address, type Address } from '@solana/kit';

import {
  buildDepositEscrowTransaction,
  buildPayTrafficFromEscrowTransaction,
  buildPayTrafficTransaction,
  payLabel,
  type Blockhashish,
  type PayConfig,
} from './pay';
import { selectReceiptsForEpoch, type TrafficReceipt } from './receipts';
import type { EpochStore } from './store';
import type { CoSigner } from './claim';

export interface ServerDeps {
  store: EpochStore;
  payConfig: PayConfig;
  getBlockhash: () => Promise<Blockhashish>;
  /** Poster co-signer for operator-paid reward claims. Absent = claims disabled (no poster key). */
  coSigner?: CoSigner;
  /** Called with the verified, deduped receipts ingested for an epoch (M6 → M4). */
  onReceipts?: (epoch: bigint, accepted: TrafficReceipt[]) => Promise<unknown> | unknown;
  /** Bearer token required on POST /receipts (relay → aggregator). Unset = open (dev only). */
  receiptsToken?: string;
}

const MAX_BODY_BYTES = 2 * 1024 * 1024; // 2 MB — bounds per-request memory (M1)

function readBody(req: import('node:http').IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    let data = '';
    let size = 0;
    req.on('data', (c) => {
      size += (c as Buffer).length;
      if (size > MAX_BODY_BYTES) {
        req.destroy();
        reject(new Error('request body too large'));
        return;
      }
      data += c;
    });
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
          res.writeHead(code, {
            'content-type': 'application/json',
            'access-control-allow-origin': '*',
            'access-control-allow-methods': 'GET, POST, OPTIONS',
            'access-control-allow-headers': 'content-type',
            'access-control-max-age': '86400',
          });
          res.end(JSON.stringify(body));
        };

        if (req.method === 'OPTIONS') {
          json(204, {});
          return;
        }

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

        if (url.pathname === '/claimable' && req.method === 'GET') {
          const operator = url.searchParams.get('operator') ?? '';
          const summary = deps.store.claimable(operator);
          json(200, {
            operator: summary.operator,
            totalAmount: summary.totalAmount.toString(),
            nodes: summary.nodes.map((node) => ({
              nodeId: node.nodeId.toString(),
              totalAmount: node.totalAmount.toString(),
              claims: node.claims.map((claim) => ({
                epoch: claim.epoch.toString(),
                operator: claim.operator,
                nodeId: claim.nodeId.toString(),
                amount: claim.amount.toString(),
                leaf: claim.leaf,
                proof: claim.proof,
              })),
            })),
          });
          return;
        }

        // Reward withdrawal (operator-paid). The cabinet builds + operator-signs a claim_rewards tx
        // (operator = fee payer, poster = empty signer slot) and POSTs the wire here. We bound the
        // payout to this node's ledger earnings, co-sign with the poster, and submit. The OPERATOR
        // pays the fee + one-time ATA rent; the poster pays nothing and only ever co-signs a claim it
        // verified. A tampered/foreign-instruction tx is rejected before the poster signs.
        if (url.pathname === '/claim' && req.method === 'POST') {
          if (!deps.coSigner) {
            json(503, { error: 'reward claims are not enabled on this aggregator' });
            return;
          }
          const body = JSON.parse((await readBody(req)) || '{}') as {
            operator?: string;
            nodeId?: string | number;
            transaction?: string;
          };
          if (!body.operator || body.nodeId === undefined || body.nodeId === null || !body.transaction) {
            json(400, { error: 'missing operator, nodeId, or transaction' });
            return;
          }
          const nodeId = BigInt(body.nodeId);
          const maxEarned = deps.store.earnedForNode(body.operator, nodeId);
          if (maxEarned <= 0n) {
            json(400, { error: 'this node has no earnings to withdraw' });
            return;
          }
          try {
            const signature = await deps.coSigner.coSign(body.transaction, {
              operator: body.operator,
              nodeId,
              maxEarned,
            });
            json(200, { signature, earnedTotal: maxEarned.toString() });
          } catch (e) {
            json(400, { error: String((e as Error)?.message ?? e) });
          }
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

        if (url.pathname === '/pay/escrow/deposit' && req.method === 'POST') {
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
          const tx = await buildDepositEscrowTransaction(
            account,
            amount,
            deps.payConfig,
            await deps.getBlockhash(),
          );
          json(200, tx);
          return;
        }

        if (url.pathname === '/pay/traffic/escrow' && req.method === 'POST') {
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
          const tx = await buildPayTrafficFromEscrowTransaction(
            account,
            amount,
            deps.payConfig,
            await deps.getBlockhash(),
          );
          json(200, tx);
          return;
        }

        if (url.pathname === '/receipts' && req.method === 'POST') {
          // Receipts move node rewards — only trusted relays may post. Without auth an anonymous
          // caller could overwrite an epoch build (erasing honest operators' proofs) or push a
          // bogus epoch. The token is a shared secret between the relay boxes and the aggregator.
          if (deps.receiptsToken && req.headers['authorization'] !== `Bearer ${deps.receiptsToken}`) {
            json(401, { error: 'unauthorized' });
            return;
          }
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
