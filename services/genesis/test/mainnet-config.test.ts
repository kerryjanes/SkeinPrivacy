import { describe, expect, it } from 'vitest';

import { loadEnv } from '../src/config';
import { runGenesis } from '../src/genesis';

function withEnv(keys: Record<string, string | undefined>, fn: () => void | Promise<void>) {
  const saved = new Map<string, string | undefined>();
  for (const key of Object.keys(keys)) {
    saved.set(key, process.env[key]);
    if (keys[key] === undefined) delete process.env[key];
    else process.env[key] = keys[key];
  }
  try {
    return fn();
  } finally {
    for (const [key, value] of saved) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
}

describe('mainnet genesis guardrails', () => {
  it('requires explicit RPC and TGE timestamp for mainnet', () => {
    withEnv(
      {
        WEFT_CLUSTER: 'mainnet-beta',
        WEFT_RPC_URL: undefined,
        WEFT_TGE_TS: undefined,
      },
      () => expect(() => loadEnv()).toThrow(/WEFT_RPC_URL/),
    );
  });

  it('refuses mainnet genesis before all custody and schedule owners are explicit', async () => {
    await expect(
      runGenesis({
        cluster: 'mainnet-beta',
        rpcUrl: 'https://api.mainnet-beta.solana.com',
        wsUrl: 'wss://api.mainnet-beta.solana.com',
        keypairPath: '/tmp/not-used.json',
        tgeTimestamp: 1n,
      }),
    ).rejects.toThrow(/missing mainnet owner override/);
  });
});
