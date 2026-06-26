import { describe, expect, it } from 'vitest';

import { loadEnv } from '../src/config';
import { loadManifest, isCurrentManifest, type Manifest } from '../src/manifest';

function withEnv(keys: Record<string, string | undefined>, fn: () => void): void {
  const saved = new Map<string, string | undefined>();
  for (const key of Object.keys(keys)) {
    saved.set(key, process.env[key]);
    if (keys[key] === undefined) delete process.env[key];
    else process.env[key] = keys[key];
  }
  try {
    fn();
  } finally {
    for (const [key, value] of saved) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
}

describe('mainnet registry provisioning guardrails', () => {
  it('requires explicit RPC for mainnet', () => {
    withEnv(
      {
        WEFT_CLUSTER: 'mainnet-beta',
        WEFT_RPC_URL: undefined,
      },
      () => expect(() => loadEnv()).toThrow(/WEFT_RPC_URL/),
    );
  });
});

describe('registry manifest guardrails', () => {
  it('loads the bundled devnet manifest for the current program id', () => {
    const manifest = loadManifest('devnet');
    expect(manifest?.registryProgram).toBe('GxhrTKKPybHZPv2MsaLovzKaq9Pq8jHmjNyRMrKZY6aH');
  });

  it('ignores stale embedded devnet manifests from previous program ids', () => {
    const stale: Manifest = {
      cluster: 'devnet',
      complete: true,
      registryProgram: '6dsqVjMmczosqNk2kaFHa33ut9ZUAwazgUagPKk5tUgd',
      registry: 'CCgRn9trB6iiCsGpyX7oGk2oXointLS1Ufd8DUgtYq9b',
      collection: 'DHos7saKjJbK93gdhvxVhSZKCywa21LHQWHWvS3ZibmD',
      merkleTree: 'CiEewSdbaFquGWffE8ZPgFh3H1RqEacCixY2CdX9mNPN',
      treeShard: 'AngRKGURcJPYnjz2gDruiHiFcgbutB4ZNyBBRcB2sUBN',
      maxDepth: 14,
    };
    expect(isCurrentManifest(stale)).toBe(false);
  });
});
