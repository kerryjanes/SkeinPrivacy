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
import { applyRelayExitBytes, exitProfileSignature } from './exitProfiles.js';
import { multiHopLink, oneHopLink } from './links.js';
import type { Settler } from './settlement.js';
import { applyConfig, pollExitUsage, pollUsage } from './xray.js';

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

const fmtWeft = (base: bigint, decimals: number): string =>
  (Number(base) / 10 ** decimals).toFixed(6);

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export class Controller {
  private lastApplied = '__uninitialized'; // force a first apply
  /** Reward-mint decimals, read from chain via loadDecimals(). $WEFT is 6 on mainnet
   *  (pump.fun); a devnet test mint may be 9. Defaults to 6 until loaded. */
  private decimals = 6;

  /** Delegated settlement engine (poster key). Absent → auto-settle disabled, manual /settle only. */
  private settler?: Settler;

  constructor(
    private cfg: NodeConfig,
    private store: Store,
    private rpc: Rpc,
  ) {}

  /** Attach the poster-signed settler so the metering loop can auto-bill escrows. */
  attachSettler(settler: Settler): void {
    this.settler = settler;
  }

  /** Fetch the reward mint's decimals once at startup so quota/price/display adapt to
   *  the actual token (same code on every cluster). Call before the metering loop. */
  async loadDecimals(): Promise<void> {
    try {
      const mi = await this.rpc
        .getAccountInfo(this.cfg.weftMint as unknown as Parameters<Rpc['getAccountInfo']>[0], {
          encoding: 'base64',
        })
        .send();
      if (mi.value) this.decimals = Buffer.from(mi.value.data[0], 'base64')[44]; // SPL Mint: decimals @ byte 44
    } catch {
      /* keep default 6 */
    }
  }

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
    this.store.sweepStalePendings(60 * 60 * 1000); // drop abandoned settle reservations >1h old
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
    u.quotaBytes = quotaBytes(bal, this.decimals).toString();
  }

  status(u: User): Status {
    const unsettled = BigInt(u.unsettledBytes);
    const quota = BigInt(u.quotaBytes);
    return {
      wallet: u.wallet,
      active: u.active,
      balanceWeft: fmtWeft(BigInt(u.balanceBaseUnits), this.decimals),
      balanceBaseUnits: u.balanceBaseUnits,
      quotaBytes: quota.toString(),
      unsettledBytes: unsettled.toString(),
      owedWeft: fmtWeft(costBaseUnits(unsettled, this.decimals), this.decimals),
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
    const isNew = !u;
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
    // Don't persist a brand-new wallet that has never funded escrow — otherwise anyone can mint
    // unbounded persisted users, each an RPC + disk write on every metering tick. A wallet becomes
    // durable state only once it has a prepaid balance; a zero-escrow probe gets an ephemeral link
    // that carries no quota anyway.
    if (isNew && BigInt(u.balanceBaseUnits) === 0n) {
      return this.status(u);
    }
    this.store.put(u);
    this.reconcile();
    return this.status(u);
  }

  /** Verify a settlement tx and clear that much of the user's metered tab. */
  async settle(wallet: string, signature: string): Promise<Status> {
    const u = this.store.get(wallet);
    if (!u) throw new Error('unknown wallet — provision first');
    const prior = this.store.payment(signature);
    if (prior?.status === 'processed') {
      // already applied on an earlier call — idempotent success, never clear the tab twice
      return this.status(u);
    }
    this.store.beginPayment(signature, wallet); // reserve (or re-adopt a pending left by a crash)
    let amount = 0n;
    try {
      let lastError: unknown;
      for (let attempt = 0; attempt < 8; attempt++) {
        try {
          ({ amount } = await verifyPayTraffic(this.rpc, signature, wallet));
          lastError = null;
          break;
        } catch (e) {
          lastError = e;
          const message = e instanceof Error ? e.message : String(e);
          if (!message.includes('transaction not found / not confirmed')) throw e;
          await sleep(1_500);
        }
      }
      if (lastError) throw lastError;
    } catch (e) {
      this.store.forgetPendingPayment(signature, wallet);
      throw e;
    }
    const paidBytes = quotaBytes(amount, this.decimals); // bytes that payment covers
    const unsettled = BigInt(u.unsettledBytes);
    u.unsettledBytes = (unsettled > paidBytes ? unsettled - paidBytes : 0n).toString();
    await this.refreshBalance(u); // escrow balance dropped by the payment
    u.active = this.computeActive(u);
    // One atomic write clears the tab AND marks the signature processed — no crash window where the
    // tab is reduced but the payment still looks unspent (re-drive would double-clear) or vice versa.
    // A concurrent double-submit returns false here and applies nothing.
    if (!this.store.applySettlement(u, signature, amount)) {
      return this.status(this.store.get(wallet) ?? u);
    }
    this.reconcile();
    return this.status(u);
  }

  /**
   * Delegated settlement pass: for every user carrying accrued (unsettled) traffic, bill it straight
   * from their prepaid escrow with the poster key — no user signature. This funds the reward vault
   * continuously (70% of the metered price) so node payouts stay solvent, and keeps access seamless:
   * the user deposits once and never has to sign a settlement.
   *
   * Billing preserves the user's access headroom exactly — settling `amount` drops both the escrow
   * balance and the unsettled tab by the same byte-equivalent, so `quota - unsettled` is unchanged.
   * A user only goes OFF once their escrow is genuinely spent (nothing left to bill).
   */
  async autoSettle(): Promise<void> {
    if (!this.settler) return;
    let changed = false;
    for (const u of this.store.all()) {
      const unsettled = BigInt(u.unsettledBytes);
      if (unsettled < this.cfg.settleMinBytes) continue; // let dust accrue; don't spend a tx on it
      const escrowBal = await escrowBalance(this.rpc, u.wallet, this.cfg.weftMint);
      const owed = costBaseUnits(unsettled, this.decimals);
      const amount = owed < escrowBal ? owed : escrowBal; // never bill more than the escrow holds
      if (amount <= 0n) continue; // no escrow to bill → the access rule takes them OFF on the next tick
      try {
        await this.settler.settle(u.wallet, amount);
      } catch (e) {
        // Leave the tab intact and retry next pass; never advance local state on a failed bill.
        console.error(`[control-plane] auto-settle ${u.wallet} failed: ${(e as Error).message}`);
        continue;
      }
      const settledBytes = quotaBytes(amount, this.decimals); // bytes that `amount` paid off
      u.unsettledBytes = (unsettled > settledBytes ? unsettled - settledBytes : 0n).toString();
      await this.refreshBalance(u); // escrow dropped by the bill
      u.active = this.computeActive(u);
      this.store.put(u);
      changed = true;
    }
    if (changed) this.reconcile();
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
    // Operator-facing ESTIMATE only. The authoritative reward basis is the relay's
    // per-node outbound measurement (exit-profiles via applyRelayExitBytes), which
    // reconciles with user billing — NOT this local counter. Shown on /node/stats.
    const servedByUsers = this.store
      .all()
      .reduce((sum, u) => sum + BigInt(u.servedBytesLifetime ?? '0'), 0n);
    const rawNodeServed = BigInt(this.store.nodeServedBytesLifetime());
    const served = servedByUsers > rawNodeServed ? servedByUsers : rawNodeServed;
    // baseline multipliers: reputation 1.0× (10000 bps), no geo/stake bonus
    const earned = math.trafficReward(served, 10_000n, 0n, 0n);
    return {
      users: this.store.all().length,
      servedBytes: served.toString(),
      earnedWeft: fmtWeft(earned, this.decimals),
      earnedBaseUnits: earned.toString(),
    };
  }

  async applyUsage(usage: Map<string, bigint>): Promise<void> {
    const rawDelta = [...usage.values()].reduce((sum, delta) => sum + delta, 0n);
    this.store.addNodeServedBytes(rawDelta);
    const users = this.store.all();
    for (const u of users) {
      const delta = usage.get(u.email) ?? 0n;
      if (delta > 0n) {
        u.unsettledBytes = (BigInt(u.unsettledBytes) + delta).toString();
        // lifetime served only ever grows — it's what the operator earns $WEFT on
        u.servedBytesLifetime = (BigInt(u.servedBytesLifetime ?? '0') + delta).toString();
      }
      await this.refreshBalance(u);
      u.active = this.computeActive(u);
    }
    this.store.putMany(users); // single write for the whole cohort — keeps the tick O(N), not O(N²)
    this.reconcile();
  }

  /** One metering cycle: fold in usage deltas, refresh balances, flip users, reconcile xray.
   *  On a relay (balancer with user-exit outbounds) also attribute forwarded bytes per exit
   *  node — the reward basis that reconciles with the user debits metered here. */
  async tick(): Promise<void> {
    await this.applyUsage(pollUsage(this.cfg));
    applyRelayExitBytes(this.cfg, pollExitUsage(this.cfg));
  }
}
