import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { nodeRegistry } from '@weft/sdk';

const here = dirname(fileURLToPath(import.meta.url));
const dir = join(here, '..', 'manifests');

export interface Manifest {
  cluster: string;
  complete: boolean;
  registryProgram: string;
  registry: string;
  collection: string;
  merkleTree: string;
  treeShard: string;
  maxDepth: number;
}

export const manifestPath = (cluster: string) => join(dir, `${cluster}.json`);

export function isCurrentManifest(m: Manifest): boolean {
  return m.registryProgram === nodeRegistry.NODE_REGISTRY_PROGRAM_ADDRESS;
}

// Public devnet registry (safe to embed) — lets the bundled node agent register without the
// manifests/ dir on disk.
const DEVNET: Manifest = {
  cluster: 'devnet',
  complete: true,
  registryProgram: 'GxhrTKKPybHZPv2MsaLovzKaq9Pq8jHmjNyRMrKZY6aH',
  registry: '6tw8x8sm18fz5jMsHfVxvPbCQm4Nf8e6gqKUn84pBjyW',
  collection: 'DLeBsmxSNB1RmcPrGSWm5J5tPqXjAKWVfqkVpCpCZdqY',
  merkleTree: '4RJP3AJ6NNoqjTCxjeJi2Erw3wwJJoHN3jpwUrSetJw5',
  treeShard: '8dvswMfvXUBg2YZSNoNijYaQBRNKqzzhyDToZq1Day8E',
  maxDepth: 14,
};

export function loadManifest(cluster: string): Manifest | null {
  const p = manifestPath(cluster);
  if (existsSync(p)) {
    const manifest = JSON.parse(readFileSync(p, 'utf8')) as Manifest;
    return isCurrentManifest(manifest) ? manifest : null;
  }
  if (cluster !== 'devnet') return null;
  return isCurrentManifest(DEVNET) ? DEVNET : null;
}
export function saveManifest(m: Manifest): void {
  mkdirSync(dir, { recursive: true });
  writeFileSync(manifestPath(m.cluster), `${JSON.stringify(m, null, 2)}\n`);
}
