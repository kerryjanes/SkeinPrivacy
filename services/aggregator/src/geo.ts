// Governance geo-demand table. The protocol pays up to +50% for traffic served
// from under-supplied regions (`SPEC.md` > Node Economics). The table maps a
// geo-region prefix (the high bits of a node's packed geohash) to a bonus in
// basis points; lookups clamp to the on-chain ceiling so a misconfigured table
// can never inflate a payout beyond what `traffic_reward` allows.

import { math } from '@weft/sdk';

export const GEO_BITS = 30;
export const GEO_MAX = ((1 << GEO_BITS) - 1) >>> 0;
export const GEO_BONUS_MAX_BPS = Number(math.GEO_BONUS_MAX_BPS);

/** Mirror of `weft_primitives::geo_region_prefix`: keep the top `chars`×5 bits. */
export function geoRegionPrefix(geo: number, chars: number): number {
  const keep = Math.min(chars, 6) * 5;
  const g = geo & GEO_MAX;
  return keep >= GEO_BITS ? g : g >>> (GEO_BITS - keep);
}

export interface GeoTable {
  /** How many geohash chars (×5 bits) the region keys are bucketed at. */
  chars: number;
  /** region-prefix (decimal string) → bonus bps. */
  bonusBps: Record<string, number>;
}

export const EMPTY_GEO_TABLE: GeoTable = { chars: 2, bonusBps: {} };

/** Bonus bps for a node's packed geo value, clamped to the on-chain ceiling. */
export function geoBonusBps(table: GeoTable, geo: number): number {
  const region = geoRegionPrefix(geo, table.chars);
  const raw = table.bonusBps[String(region)] ?? 0;
  return Math.max(0, Math.min(raw, GEO_BONUS_MAX_BPS));
}
