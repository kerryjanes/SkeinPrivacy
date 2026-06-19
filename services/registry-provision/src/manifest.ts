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
export function loadManifest(cluster: string): Manifest | null {
  const p = manifestPath(cluster);
  return existsSync(p) ? (JSON.parse(readFileSync(p, 'utf8')) as Manifest) : null;
}
export function saveManifest(m: Manifest): void {
  mkdirSync(dir, { recursive: true });
  writeFileSync(manifestPath(m.cluster), `${JSON.stringify(m, null, 2)}\n`);
}
