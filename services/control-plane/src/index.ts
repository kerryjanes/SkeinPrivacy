#!/usr/bin/env -S tsx
// Weft control plane — token-gated VPN access for one node.
//
// Boots the HTTP API + the metering loop. The loop is what enforces the economics: every pollMs
// it folds each user's new traffic into their tab, re-reads prepaid escrow balances, and turns links
// on/off so a user only stays connected while their prepaid balance covers what they've used.

import { readFileSync } from 'node:fs';
import { loadConfig } from './config.js';
import { Store } from './store.js';

// launchd (macOS) can't pass an EnvironmentFile, so honor WEFT_ENVFILE: load KEY=VALUE lines
// into process.env before reading config. (systemd uses EnvironmentFile directly.)
const envFile = process.env.WEFT_ENVFILE;
if (envFile) {
  for (const line of readFileSync(envFile, 'utf8').split('\n')) {
    const m = line.match(/^\s*([A-Z0-9_]+)\s*=\s*(.*)\s*$/);
    if (m && process.env[m[1]] === undefined) process.env[m[1]] = m[2];
  }
}
import { rpc } from './chain.js';
import { Controller } from './controller.js';
import { SolPay } from './solpay.js';
import { publishOwnExitProfile } from './exitProfiles.js';
import { Faucet } from './faucet.js';
import { startServer } from './server.js';

const cfg = loadConfig();
const store = new Store(cfg.storePath);

// Optional SOL payment path. Off unless WEFT_SOL_PAYMENTS=1 with a collection wallet set —
// when off, solPay is undefined and the controller uses escrow quota only (launch default).
const solPay =
  process.env.WEFT_SOL_PAYMENTS === '1' && process.env.WEFT_SOL_COLLECTION
    ? new SolPay({
        collection: process.env.WEFT_SOL_COLLECTION,
        pricePerGbLamports: BigInt(process.env.WEFT_SOL_PRICE_PER_GB_LAMPORTS ?? '10000000'), // 0.01 SOL/GB
        minLamports: BigInt(process.env.WEFT_SOL_MIN_LAMPORTS ?? '1000000'), // 0.001 SOL
        storePath: process.env.WEFT_SOL_STORE ?? '/var/lib/weft/solpay.json',
      })
    : undefined;
const ctrl = new Controller(cfg, store, rpc(cfg.rpcUrl), solPay);
const faucet = cfg.faucetKeypairPath
  ? new Faucet(
      cfg.rpcUrl,
      cfg.wsUrl,
      cfg.faucetKeypairPath,
      cfg.weftMint,
      cfg.faucetAmount,
      cfg.faucetSolLamports,
      cfg.faucetCooldownMs,
    )
  : undefined;

await ctrl.loadDecimals(); // read reward-mint decimals so quota/price adapt to the token
ctrl.bootstrap(); // sync xray to saved state on boot
startServer(cfg, ctrl, faucet);

async function profileHeartbeat(): Promise<void> {
  if (!cfg.relayProfileUrl) return;
  for (;;) {
    try {
      await publishOwnExitProfile(cfg, BigInt(ctrl.nodeStats().servedBytes));
    } catch (e) {
      // eslint-disable-next-line no-console
      console.error('relay profile publish error:', (e as Error).message);
    }
    await new Promise((r) => setTimeout(r, Math.min(30000, Math.max(5000, cfg.exitProfileTtlMs / 2))));
  }
}

async function loop(): Promise<void> {
  for (;;) {
    try {
      await ctrl.tick();
    } catch (e) {
      // eslint-disable-next-line no-console
      console.error('tick error:', (e as Error).message);
    }
    await new Promise((r) => setTimeout(r, cfg.pollMs));
  }
}
void profileHeartbeat();
void loop();
