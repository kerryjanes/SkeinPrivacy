#!/usr/bin/env -S tsx
// Self-contained node-registration agent, run by scripts/run-node.sh — registers the node on-chain
// under the OPERATOR'S OWN WALLET (the one you pass in), so the node shows up under that wallet in
// the cabinet and earns to it. Idempotent: if the node is already registered, it does nothing
// (restarting never re-pays). Stopping the node never touches the chain — the relay liveness drops
// it from the live list.
//
// Env: WEFT_KEYPAIR (your wallet keypair JSON — required) · WEFT_NODE_ENDPOINT (relay:port) ·
//      WEFT_GEO (packed) · WEFT_RPC
import { existsSync, readFileSync } from 'node:fs';
import { createKeyPairSignerFromBytes } from '@solana/kit';
import { becomeNode, deriveNodeId } from './becomeNode';
import { loadEnv, nodePda } from './config';
import { connect } from './kit';

const env = loadEnv();
const endpoint = process.env.WEFT_NODE_ENDPOINT;
const geo = Number(process.env.WEFT_GEO ?? '0');
if (!endpoint) {
  console.error('WEFT_NODE_ENDPOINT required');
  process.exit(1);
}
if (!existsSync(env.keypairPath)) {
  console.error(
    `wallet keypair not found at ${env.keypairPath}\n` +
      'Pass your wallet: WEFT_KEYPAIR=/path/to/wallet.json (the node registers + earns under it).',
  );
  process.exit(1);
}

const operator = await createKeyPairSignerFromBytes(
  Uint8Array.from(JSON.parse(readFileSync(env.keypairPath, 'utf8')) as number[]),
);
const conn = connect(env.rpcUrl, env.wsUrl);
console.log('node operator (your wallet):', operator.address);

// The operator's own SOL pays the one-time registration (~0.0015 SOL rent + fee). No faucet.
const sol = Number((await conn.rpc.getBalance(operator.address).send()).value);
if (sol < 1_000_000) {
  console.error(
    `wallet ${operator.address} has too little SOL for the one-time registration.\n` +
      `Fund it (devnet: solana airdrop 1 ${operator.address} -u devnet) and re-run.`,
  );
  process.exit(1);
}

// Register iff not already registered (idempotent — restart never re-pays). Pre-check the node
// PDA so we don't even send a tx (and don't pay a fee) when it already exists.
const nodeId = deriveNodeId(operator.address, endpoint);
const node = await nodePda(operator.address, nodeId);
const exists = (await conn.rpc.getAccountInfo(node, { encoding: 'base64' }).send()).value;
if (exists) {
  console.log(`already registered under your wallet (node ${nodeId}) — nothing to pay.`);
} else {
  const reg = await becomeNode(conn, operator, { endpoint, geo }, env.cluster);
  console.log(`registered node ${reg.nodeId} · tx ${reg.signature.slice(0, 16)}…`);
}
