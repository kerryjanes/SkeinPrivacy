// The control loop. Owns the user store + chain reads + xray surface, and ties them together:
//
//   • provision(wallet) — mint a per-user link, gated by the wallet's prepaid escrow balance
//   • settle(wallet,tx) — verify an on-chain settlement and clear that much of the user's tab
//   • tick()            — meter usage, refresh balances, flip users on/off, reconcile xray
//
// Access rule (the whole point): a user is ON iff their unsettled consumption is still within
// what their prepaid escrow can pay for (`unsettledBytes < quotaBytes`). Consume past it → OFF.
// Settle from escrow (or legacy pay_traffic) / top up $WEFT → quota rises / tab clears → ON again.

import { randomUUID } from 'node:crypto';
import type { NodeConfig } from './config.js';
import type { Store, User } from './store.js';
import { math } from '@weft/sdk';
import { costBaseUnits, escrowBalance, quotaBytes, verifyPayTraffic, type Rpc } from './chain.js';
import { exitProfileSignature } from './exitProfiles.js';
import { multiHopLink, oneHopLink } from './links.js';
import { applyConfig, pollUsage } from './xray.js';

export interface Status {
  wallet: string;
  active: boolean;
  balanceWeft: string; // prepaid escrow balance, kept under the legacy field name for API stability
  balanceBaseUnits: string;
  quotaBytes: string;
  unsettledBytes: string;
  owedWeft: string; // what the user owes for unsettled traffic (settle this to clear it)
  remainingBytes: string; // headroom before cutoff
  links: { oneHop: string; multiHop: string };
}

const fmtWeft = (base: bigint): string => (Number(base) / 1e9).toFixed(6);

export class Controller {
  private lastApplied = '__uninitialized'; // force a first apply

  constructor(
    private cfg: NodeConfig,
    private store: Store,
    private rpc: Rpc,
  ) {}

  /** Render + reload xray only when the active user set actually changed. */
  reconcile(): void {
    const active = this.store.all().filter((u) => u.active);
    const sig = active
      .map((u) => u.uuid)
      .sort()
      .join(',');
    const profileSig = exitProfileSignature(this.cfg);
    const combinedSig = `${sig}|${profileSig}`;
    if (combinedSig === this.lastApplied) return;
    applyConfig(this.cfg, active);
    this.lastApplied = combinedSig;
  }

  /** Apply whatever the store says on boot (so a restart re-syncs xray to the saved state). */
  bootstrap(): void {
    this.reconcile();
    if (this.lastApplied === '__uninitialized') {
      // no active users → still need a valid config with just the founder present
      applyConfig(this.cfg, []);
      this.lastApplied = `|${exitProfileSignature(this.cfg)}`;
    }
  }

  private computeActive(u: User): boolean {
    return BigInt(u.unsettledBytes) < BigInt(u.quotaBytes);
  }

  private async refreshBalance(u: User): Promise<void> {
    const bal = await escrowBalance(this.rpc, u.wallet, this.cfg.weftMint);
    u.balanceBaseUnits = bal.toString();
    u.quotaBytes = quotaBytes(bal).toString();
  }

  status(u: User): Status {
    const unsettled = BigInt(u.unsettledBytes);
    const quota = BigInt(u.quotaBytes);
    return {
      wallet: u.wallet,
      active: u.active,
      balanceWeft: fmtWeft(BigInt(u.balanceBaseUnits)),
      balanceBaseUnits: u.balanceBaseUnits,
      quotaBytes: quota.toString(),
      unsettledBytes: unsettled.toString(),
      owedWeft: fmtWeft(costBaseUnits(unsettled)),
      remainingBytes: (quota > unsettled ? quota - unsettled : 0n).toString(),
      links: {
        oneHop: oneHopLink(this.cfg, u.uuid),
        // 1-hop-only home node → no multihop link (multihop is served over Tor by infra nodes).
        multiHop: this.cfg.hopnPort > 0 ? multiHopLink(this.cfg, u.uuid) : '',
      },
    };
  }

  /** Create the user if new, refresh their quota from chain, and return their personal links. */
  async provision(wallet: string): Promise<Status> {
    let u = this.store.get(wallet);
    if (!u) {
      u = {
        wallet,
        uuid: randomUUID(),
        email: wallet, // unique stats key
        unsettledBytes: '0',
        servedBytesLifetime: '0',
        balanceBaseUnits: '0',
        quotaBytes: '0',
        active: false,
        createdAt: Date.now(),
      };
    }
    await this.refreshBalance(u);
    u.active = this.computeActive(u);
    this.store.put(u);
    this.reconcile();
    return this.status(u);
  }

  /** Verify a settlement tx and clear that much of the user's metered tab. */
  async settle(wallet: string, signature: string): Promise<Status> {
    const u = this.store.get(wallet);
    if (!u) throw new Error('unknown wallet — provision first');
    this.store.beginPayment(signature, wallet);
    let amount: bigint;
    try {
      ({ amount } = await verifyPayTraffic(this.rpc, signature, wallet));
    } catch (e) {
      this.store.forgetPendingPayment(signature, wallet);
      throw e;
    }
    const paidBytes = quotaBytes(amount); // bytes that payment covers
    const unsettled = BigInt(u.unsettledBytes);
    u.unsettledBytes = (unsettled > paidBytes ? unsettled - paidBytes : 0n).toString();
    await this.refreshBalance(u); // escrow balance dropped by the payment
    u.active = this.computeActive(u);
    this.store.put(u);
    this.store.completePayment(signature, wallet, amount);
    this.reconcile();
    return this.status(u);
  }

  /**
   * What this node has earned: total bytes served across all users → the $WEFT reward the
   * operator is owed. Uses the same `trafficReward` math the aggregator applies on-chain (here at
   * baseline reputation; the on-chain reputation/geo/stake multipliers are applied at settlement).
   * This is the per-node served-traffic total the reward pipeline (aggregator → claim) consumes.
   */
  nodeStats(): {
    users: number;
    servedBytes: string;
    earnedWeft: string;
    earnedBaseUnits: string;
  } {
    const served = this.store
      .all()
      .reduce((sum, u) => sum + BigInt(u.servedBytesLifetime ?? '0'), 0n);
    // baseline multipliers: reputation 1.0× (10000 bps), no geo/stake bonus
    const earned = math.trafficReward(served, 10_000n, 0n, 0n);
    return {
      users: this.store.all().length,
      servedBytes: served.toString(),
      earnedWeft: fmtWeft(earned),
      earnedBaseUnits: earned.toString(),
    };
  }

  /** One metering cycle: fold in usage deltas, refresh balances, flip users, reconcile xray. */
  async tick(): Promise<void> {
    const usage = pollUsage(this.cfg);
    for (const u of this.store.all()) {
      const delta = usage.get(u.email) ?? 0n;
      if (delta > 0n) {
        u.unsettledBytes = (BigInt(u.unsettledBytes) + delta).toString();
        // lifetime served only ever grows — it's what the operator earns $WEFT on
        u.servedBytesLifetime = (BigInt(u.servedBytesLifetime ?? '0') + delta).toString();
      }
      await this.refreshBalance(u);
      u.active = this.computeActive(u);
      this.store.put(u);
    }
    this.reconcile();
  }
}
