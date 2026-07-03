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
import { randomBytes } from 'node:crypto';
import { ed25519 } from '@noble/curves/ed25519';
import { address, getAddressEncoder, type Address } from '@solana/kit';

import {
  buildDepositEscrowTransaction,
  buildPayTrafficFromEscrowTransaction,
  buildPayTrafficTransaction,
  payLabel,
  type Blockhashish,
  type PayConfig,
} from './pay';
import { selectReceiptsForEpoch, type TrafficReceipt } from './receipts';
import type { EpochStore, PayoutStore } from './store';
import type { PayoutBackend } from './payout';

const WITHDRAW_CHALLENGE_TTL_MS = 5 * 60 * 1000;
const addressEncoder = getAddressEncoder();

export interface ServerDeps {
  store: EpochStore;
  payConfig: PayConfig;
  getBlockhash: () => Promise<Blockhashish>;
  /** Called with the verified, deduped receipts ingested for an epoch (M6 → M4). */
  onReceipts?: (epoch: bigint, accepted: TrafficReceipt[]) => Promise<unknown> | unknown;
  payoutStore?: PayoutStore;
  payout?: PayoutBackend;
  payoutReserve?: bigint;
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

function earnedSummary(store: EpochStore, payouts: PayoutStore | undefined, operator: string) {
  const summary = store.claimable(operator);
  const nodes = summary.nodes.map((node) => {
    const paid = payouts?.paid(operator, node.nodeId) ?? 0n;
    const withdrawable = node.totalAmount > paid ? node.totalAmount - paid : 0n;
    return {
      nodeId: node.nodeId,
      earned: node.totalAmount,
      paid,
      withdrawable,
    };
  });
  return {
    operator,
    totalEarned: nodes.reduce((sum, node) => sum + node.earned, 0n),
    totalPaid: nodes.reduce((sum, node) => sum + node.paid, 0n),
    withdrawable: nodes.reduce((sum, node) => sum + node.withdrawable, 0n),
    nodes,
  };
}

interface WithdrawChallenge {
  operator: string;
  nodeId: string;
  nonce: string;
  expiresAt: number;
  message: string;
}

function challengeKey(operator: string, nodeId: string): string {
  return `${operator}:${nodeId || 'all'}`;
}

function buildWithdrawMessage(challenge: Omit<WithdrawChallenge, 'message'>): string {
  return [
    'Weft earned withdrawal',
    `operator: ${challenge.operator}`,
    `nodeId: ${challenge.nodeId || 'all'}`,
    `nonce: ${challenge.nonce}`,
    `expiresAt: ${challenge.expiresAt}`,
  ].join('\n');
}

function verifyWithdrawalSignature(
  operator: string,
  message: string,
  signatureBase64: string,
): boolean {
  const publicKey = new Uint8Array(addressEncoder.encode(address(operator) as Address));
  const signature = Buffer.from(signatureBase64, 'base64');
  if (signature.length !== 64) return false;
  return ed25519.verify(signature, Buffer.from(message, 'utf8'), publicKey);
}

export function createAggregatorServer(deps: ServerDeps): Server {
  const withdrawChallenges = new Map<string, WithdrawChallenge>();

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

        if (url.pathname === '/earned' && req.method === 'GET') {
          const operator = url.searchParams.get('operator') ?? '';
          const summary = earnedSummary(deps.store, deps.payoutStore, operator);
          json(200, {
            operator: summary.operator,
            totalEarned: summary.totalEarned.toString(),
            totalPaid: summary.totalPaid.toString(),
            withdrawable: summary.withdrawable.toString(),
            nodes: summary.nodes.map((node) => ({
              nodeId: node.nodeId.toString(),
              earned: node.earned.toString(),
              paid: node.paid.toString(),
              withdrawable: node.withdrawable.toString(),
            })),
          });
          return;
        }

        if (url.pathname === '/withdraw-earned/challenge' && req.method === 'POST') {
          if (!deps.payout || !deps.payoutStore) {
            json(404, { error: 'earned payout disabled' });
            return;
          }
          const body = JSON.parse((await readBody(req)) || '{}') as {
            operator?: string;
            nodeId?: string;
          };
          const operator = body.operator ? String(address(body.operator)) : '';
          if (!operator) {
            json(400, { error: 'operator required' });
            return;
          }
          const nodeId = body.nodeId ? BigInt(body.nodeId).toString() : '';
          const expiresAt = Date.now() + WITHDRAW_CHALLENGE_TTL_MS;
          const challenge: Omit<WithdrawChallenge, 'message'> = {
            operator,
            nodeId,
            nonce: randomBytes(16).toString('hex'),
            expiresAt,
          };
          const full = { ...challenge, message: buildWithdrawMessage(challenge) };
          withdrawChallenges.set(challengeKey(operator, nodeId), full);
          json(200, {
            operator,
            nodeId,
            message: full.message,
            nonce: full.nonce,
            expiresAt: full.expiresAt,
          });
          return;
        }

        if (url.pathname === '/withdraw-earned' && req.method === 'POST') {
          if (!deps.payout || !deps.payoutStore) {
            json(404, { error: 'earned payout disabled' });
            return;
          }
          const body = JSON.parse((await readBody(req)) || '{}') as {
            operator?: string;
            nodeId?: string;
            message?: string;
            signature?: string;
          };
          const operator = body.operator ? String(address(body.operator)) : '';
          if (!operator) {
            json(400, { error: 'operator required' });
            return;
          }
          const nodeId = body.nodeId ? BigInt(body.nodeId).toString() : '';
          const challenge = withdrawChallenges.get(challengeKey(operator, nodeId));
          if (
            !challenge ||
            challenge.message !== body.message ||
            Date.now() > challenge.expiresAt
          ) {
            json(401, { error: 'withdraw signature challenge expired or missing' });
            return;
          }
          if (
            !body.signature ||
            !verifyWithdrawalSignature(operator, body.message, body.signature)
          ) {
            json(401, { error: 'invalid withdraw signature' });
            return;
          }
          withdrawChallenges.delete(challengeKey(operator, nodeId));

          const onlyNodeId = nodeId ? BigInt(nodeId) : null;
          const summary = earnedSummary(deps.store, deps.payoutStore, operator);
          const payableNodes = summary.nodes.filter(
            (node) => node.withdrawable > 0n && (onlyNodeId === null || node.nodeId === onlyNodeId),
          );
          const amount = payableNodes.reduce((sum, node) => sum + node.withdrawable, 0n);
          if (amount <= 0n) {
            json(400, { error: 'nothing to withdraw' });
            return;
          }
          const reserve = deps.payoutReserve ?? 0n;
          const available = await deps.payout.availableBalance();
          if (available < amount + reserve) {
            json(503, {
              error: 'payout wallet balance cannot cover withdrawal plus reserve',
              available: available.toString(),
              required: (amount + reserve).toString(),
              amount: amount.toString(),
              reserve: reserve.toString(),
            });
            return;
          }
          const { signature } = await deps.payout.pay(operator, amount);
          for (const node of payableNodes) {
            deps.payoutStore.record(operator, node.nodeId, node.withdrawable, signature);
          }
          json(200, { signature, amount: amount.toString() });
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
