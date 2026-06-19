// Deployment manifest: records every genesis address + amount so the run is
// auditable and resumable. Amounts are decimal strings (1e18 exceeds JS number).

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const manifestsDir = join(here, '..', 'manifests');

export interface ScheduleRecord {
  schedule: string;
  vault: string;
  beneficiary: string;
  authority: string;
  amount: string;
}

export interface Manifest {
  cluster: string;
  complete: boolean;
  tgeTimestamp: string;
  weftMint: string;
  mintAuthorityRetired: boolean;
  vestingProgramId: string;
  custody: Record<
    'treasury' | 'emissions' | 'idoTge',
    { owner: string; ata: string; amount: string }
  >;
  schedules: Record<string, ScheduleRecord>;
}

export function manifestPath(cluster: string): string {
  return join(manifestsDir, `${cluster}.json`);
}

export function loadManifest(cluster: string): Manifest | null {
  const p = manifestPath(cluster);
  if (!existsSync(p)) return null;
  return JSON.parse(readFileSync(p, 'utf8')) as Manifest;
}

export function saveManifest(m: Manifest): void {
  mkdirSync(manifestsDir, { recursive: true });
  writeFileSync(manifestPath(m.cluster), `${JSON.stringify(m, null, 2)}\n`);
}
