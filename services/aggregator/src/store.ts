// In-memory epoch/proof store. Holds the built epochs the aggregator has posted
// so `GET /proof?epoch&operator&nodeId` can hand a claimant exactly the proof
// the on-chain `claim` will accept. (Production would persist this; the shape is
// deliberately serializable.)

import type { EpochBuild, EpochEntry } from './rewards';

export class EpochStore {
  private byEpoch = new Map<string, EpochBuild>();

  put(build: EpochBuild): void {
    this.byEpoch.set(build.epoch.toString(), build);
  }

  get(epoch: bigint): EpochBuild | undefined {
    return this.byEpoch.get(epoch.toString());
  }

  proof(epoch: bigint, operator: string, nodeId: bigint): EpochEntry | undefined {
    const build = this.get(epoch);
    if (!build) return undefined;
    return build.entries.find((e) => e.operator === operator && e.nodeId === nodeId);
  }

  epochs(): bigint[] {
    return [...this.byEpoch.values()].map((b) => b.epoch);
  }
}
