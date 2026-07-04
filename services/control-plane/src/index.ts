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
import { publishOwnExitProfile } from './exitProfiles.js';
import { Faucet } from './faucet.js';
import { startServer } from './server.js';
import { weft } from '@weft/sdk';

const cfg = loadConfig();
const store = new Store(cfg.storePath);
const ctrl = new Controller(cfg, store, rpc(cfg.rpcUrl));
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

// Guard: WEFT_MINT (operator config) must match the on-chain distributor's reward mint. If they
// disagree, the node would meter quota against the wrong token. Only checked once the distributor
// exists — a fresh, not-yet-initialized cluster has nothing to compare against.
async function assertMintMatchesDistributor(): Promise<void> {
  const [distributor] = await weft.findDistributorPda();
  let di;
  try {
    di = await rpc(cfg.rpcUrl).getAccountInfo(distributor, { encoding: 'base64' }).send();
  } catch (e) {
    console.error('[control-plane] could not verify WEFT_MINT vs distributor:', (e as Error).message);
    return; // don't block boot on a transient RPC error
  }
  if (!di.value) return; // distributor not initialized yet — nothing to compare
  const onchainMint = String(
    weft.getDistributorDecoder().decode(Buffer.from(di.value.data[0], 'base64')).rewardMint,
  );
  if (onchainMint !== cfg.weftMint) {
    throw new Error(
      `WEFT_MINT ${cfg.weftMint} disagrees with the on-chain distributor reward mint ${onchainMint} — ` +
        `refusing to meter against the wrong token. Fix WEFT_MINT.`,
    );
  }
}

await assertMintMatchesDistributor();
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
    await new Promise((r) =>
      setTimeout(r, Math.min(30000, Math.max(5000, cfg.exitProfileTtlMs / 2))),
    );
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
