import { describe, expect, it } from 'vitest';
import type { Address } from '@solana/kit';

import { buildProfileByteTotals } from '../src/profileTotals';

describe('profile byte totals', () => {
  it('maps live relay profiles to registered nodes and returns only unsettled byte deltas', () => {
    const profiles = [
      {
        host: 'vpn.weftnetwork.net',
        port: 20026,
        servedBytesLifetime: '1500',
      },
      {
        host: 'vpn.weftnetwork.net',
        port: 20027,
        servedBytesLifetime: '9000',
      },
    ];
    const nodes = [
      {
        operator: 'operator-a' as Address,
        nodeId: 11n,
        endpointHash: '218cc5df43869a66467a7f3e67799beb9fc0ad30749347a533d7839a116505ba',
        reputationBps: 10_000,
        geo: 0,
        stake: 0n,
      },
    ];
    const settled = { 'vpn.weftnetwork.net:20026': '1000' };

    const result = buildProfileByteTotals(profiles, nodes, settled);

    expect(result.totals).toEqual([{ operator: 'operator-a', nodeId: 11n, bytes: 500n }]);
    expect(result.nextSettled).toEqual({ 'vpn.weftnetwork.net:20026': '1500' });
  });
});
