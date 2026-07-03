import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { buildEpochFromByteTotals } from '../src/rewards';
import { EpochStore } from '../src/store';
import { makeSigner } from './helpers';

describe('epoch store persistence', () => {
  it('persists built epoch proofs across aggregator restarts', () => {
    const dir = mkdtempSync(join(tmpdir(), 'weft-epochs-'));
    try {
      const path = join(dir, 'epochs.json');
      const operator = makeSigner().address;
      const build = buildEpochFromByteTotals(
        9n,
        [{ operator, nodeId: 7n, bytes: 1_000_000_000n }],
        [{ operator, nodeId: 7n, endpointHash: 'abc', reputationBps: 10_000, geo: 0, stake: 0n }],
        { minStakeToEarn: 0n },
      );
      new EpochStore(path).put(build);

      const restored = new EpochStore(path);

      expect(restored.proof(9n, operator, 7n)?.amount).toBe(build.entries[0].amount);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('reports the highest persisted epoch so auto-settlement never reuses old ids', () => {
    const dir = mkdtempSync(join(tmpdir(), 'weft-epochs-'));
    try {
      const path = join(dir, 'epochs.json');
      const operator = makeSigner().address;
      const store = new EpochStore(path);
      for (const epoch of [2n, 9n, 4n]) {
        store.put(
          buildEpochFromByteTotals(
            epoch,
            [{ operator, nodeId: 7n, bytes: 1_000_000n }],
            [
              {
                operator,
                nodeId: 7n,
                endpointHash: 'abc',
                reputationBps: 10_000,
                geo: 0,
                stake: 0n,
              },
            ],
            { minStakeToEarn: 0n },
          ),
        );
      }

      expect(new EpochStore(path).maxEpoch()).toBe(9n);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
