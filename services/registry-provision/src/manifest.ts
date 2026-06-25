import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

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

// Public devnet registry (safe to embed) — lets the bundled node agent register without the
// manifests/ dir on disk.
const DEVNET: Manifest = {
  cluster: 'devnet',
  complete: true,
  registryProgram: '6dsqVjMmczosqNk2kaFHa33ut9ZUAwazgUagPKk5tUgd',
  registry: 'CCgRn9trB6iiCsGpyX7oGk2oXointLS1Ufd8DUgtYq9b',
  collection: 'DHos7saKjJbK93gdhvxVhSZKCywa21LHQWHWvS3ZibmD',
  merkleTree: 'CiEewSdbaFquGWffE8ZPgFh3H1RqEacCixY2CdX9mNPN',
  treeShard: 'AngRKGURcJPYnjz2gDruiHiFcgbutB4ZNyBBRcB2sUBN',
  maxDepth: 14,
};

export function loadManifest(cluster: string): Manifest | null {
  const p = manifestPath(cluster);
  if (existsSync(p)) return JSON.parse(readFileSync(p, 'utf8')) as Manifest;
  return cluster === 'devnet' ? DEVNET : null;
}
export function saveManifest(m: Manifest): void {
  mkdirSync(dir, { recursive: true });
  writeFileSync(manifestPath(m.cluster), `${JSON.stringify(m, null, 2)}\n`);
}
