#!/usr/bin/env -S tsx
// Self-contained node-registration agent, run by scripts/run-node.sh — so a node operator never
// touches the website to register. It: (1) loads or generates the node's operator keypair, (2) on
// devnet, tops it up from the relay faucet so it can pay the one-time registration, (3) registers
// the node on-chain IF not already registered (idempotent — restarting never re-pays).
//
// Env: WEFT_NODE_ENDPOINT (relay:port) · WEFT_GEO (packed) · WEFT_KEYPAIR (path) ·
//      WEFT_RPC · WEFT_FAUCET_URL (relay control plane, for devnet SOL)
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { createKeyPairSignerFromBytes } from '@solana/kit';
import { ed25519 } from '@noble/curves/ed25519';
import { becomeNode } from './becomeNode';
import { loadEnv } from './config';
import { connect } from './kit';

const env = loadEnv();
const endpoint = process.env.WEFT_NODE_ENDPOINT;
const geo = Number(process.env.WEFT_GEO ?? '0');
const faucetUrl = process.env.WEFT_FAUCET_URL;
if (!endpoint) {
  console.error('WEFT_NODE_ENDPOINT required');
  process.exit(1);
}

// 1) load or generate the operator keypair (Solana 64-byte format: seed‖pubkey)
const kpPath = env.keypairPath;
if (!existsSync(kpPath)) {
  const seed = ed25519.utils.randomSecretKey();
  const pub = ed25519.getPublicKey(seed);
  const bytes = new Uint8Array(64);
  bytes.set(seed, 0);
  bytes.set(pub, 32);
  mkdirSync(dirname(kpPath), { recursive: true });
  writeFileSync(kpPath, JSON.stringify([...bytes]));
  console.log('generated node operator key →', kpPath);
}
const kpBytes = Uint8Array.from(JSON.parse(readFileSync(kpPath, 'utf8')) as number[]);
const operator = await createKeyPairSignerFromBytes(kpBytes);
const conn = connect(env.rpcUrl, env.wsUrl);
console.log('node operator:', operator.address);

// 2) ensure the key has SOL for the (one-time) registration — devnet: pull from the relay faucet
const sol = async () => Number((await conn.rpc.getBalance(operator.address).send()).value);
if ((await sol()) < 3_000_000 && faucetUrl) {
  console.log('funding from relay faucet…');
  try {
    await fetch(`${faucetUrl}/faucet`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ wallet: operator.address }),
    });
    for (let i = 0; i < 20 && (await sol()) < 3_000_000; i++)
      await new Promise((r) => setTimeout(r, 2000));
  } catch {
    /* faucet down — operator must fund manually */
  }
}
if ((await sol()) < 1_000_000) {
  console.error(`operator has no SOL — fund ${operator.address} (devnet) and re-run`);
  process.exit(1);
}

// 3) register iff not already registered (idempotent)
try {
  const reg = await becomeNode(conn, operator, { endpoint, geo }, env.cluster);
  console.log(`registered node ${reg.nodeId} · tx ${reg.signature.slice(0, 16)}…`);
} catch (e) {
  const msg = (e as Error).message;
  if (/already in use|already exists/i.test(msg))
    console.log('already registered — nothing to pay.');
  else throw e;
}
