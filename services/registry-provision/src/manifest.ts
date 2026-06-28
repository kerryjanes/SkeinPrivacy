import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { weft } from '@weft/sdk';

const here = dirname(fileURLToPath(import.meta.url));
const dir = join(here, '..', 'manifests');

export interface Manifest {
  cluster: string;
  complete: boolean;
  registryProgram: string;
}

export const manifestPath = (cluster: string) => join(dir, `${cluster}.json`);

export function isCurrentManifest(m: Manifest): boolean {
  return m.registryProgram === weft.WEFT_PROGRAM_ADDRESS;
}

const DEVNET: Manifest = {
  cluster: 'devnet',
  complete: true,
  registryProgram: weft.WEFT_PROGRAM_ADDRESS,
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
