#!/usr/bin/env -S tsx
// Weft control plane — token-gated VPN access for one node.
//
// Boots the HTTP API + the metering loop. The loop is what enforces the economics: every pollMs
// it folds each user's new traffic into their tab, re-reads $WEFT balances, and turns links
// on/off so a user only stays connected while their balance covers what they've used.

import { loadConfig } from './config.js';
import { Store } from './store.js';
import { rpc } from './chain.js';
import { Controller } from './controller.js';
import { startServer } from './server.js';

const cfg = loadConfig();
const store = new Store(cfg.storePath);
const ctrl = new Controller(cfg, store, rpc(cfg.rpcUrl));

ctrl.bootstrap(); // sync xray to saved state on boot
startServer(cfg, ctrl);

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
void loop();
