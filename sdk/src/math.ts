// BigInt mirror of the on-chain Weft reward/settlement math
// (`crates/weft-primitives`). Every function here MUST stay byte-identical to
// its Rust counterpart — the same floor-division order, the same clamps, the
// same SHA-256 leaf encoding — so the off-chain aggregator's amounts and merkle
// leaves match exactly what the on-chain `claim` accepts. Enforced by the
// cross-language parity test (`sdk/test/math.parity.test.ts`), which replays
// golden vectors emitted by `cargo run -p weft-primitives --example golden`.

import { sha256 } from '@noble/hashes/sha2';

// ---- constants (mirror weft-primitives) ----
export const BPS = 10_000n;
export const ONE_WEFT = 1_000_000n;
export const USER_PRICE_PER_GB = 1_000n * ONE_WEFT; // 1000 WEFT/GB
export const NODE_REWARD_RATE_PER_GB = 700n * ONE_WEFT; // funded by the default 70% node share
export const BYTES_PER_GB = 1_000_000_000n;
export const REPUTATION_MIN_BPS = 5_000n;
export const REPUTATION_MAX_BPS = 20_000n;
export const GEO_BONUS_MAX_BPS = 5_000n;
export const STAKING_BONUS_BPS = 2_000n;
export const STAKING_BONUS_THRESHOLD = 10_000n * ONE_WEFT;
export const SPLIT_NODES_BPS = 7_000n;
export const SPLIT_BURN_BPS = 2_000n;
export const U64_MAX = 2n ** 64n - 1n;

function clamp(x: bigint, lo: bigint, hi: bigint): bigint {
  return x < lo ? lo : x > hi ? hi : x;
}

export function clampReputationBps(reputationBps: bigint): bigint {
  return clamp(reputationBps, REPUTATION_MIN_BPS, REPUTATION_MAX_BPS);
}

export function clampGeoBonusBps(geoBonusBps: bigint): bigint {
  return geoBonusBps > GEO_BONUS_MAX_BPS ? GEO_BONUS_MAX_BPS : geoBonusBps;
}

export function stakingBonusForStake(stakedBaseUnits: bigint): bigint {
  return stakedBaseUnits >= STAKING_BONUS_THRESHOLD ? STAKING_BONUS_BPS : 0n;
}

/**
 * Reward (in $WEFT base units) for `bytes` of relayed traffic. Mirrors
 * `weft_primitives::traffic_reward` exactly, including the left-to-right
 * floor-division order and the saturation at u64::MAX.
 */
export function trafficReward(
  bytes: bigint,
  reputationBps: bigint,
  geoBonusBps: bigint,
  stakingBonusBps: bigint,
): bigint {
  const base = (NODE_REWARD_RATE_PER_GB * bytes) / BYTES_PER_GB;
  const reputation = clampReputationBps(reputationBps);
  const geo = BPS + clampGeoBonusBps(geoBonusBps);
  const staking = BPS + (stakingBonusBps > STAKING_BONUS_BPS ? STAKING_BONUS_BPS : stakingBonusBps);
  // Same left-to-right floor-division order as Rust: base·rep/BPS·geo/BPS·stk/BPS.
  const reward = (((((base * reputation) / BPS) * geo) / BPS) * staking) / BPS;
  return reward > U64_MAX ? U64_MAX : reward;
}

/** Maximum cold-start bonus (bps): +100%. Mirrors `BOOTSTRAP_BONUS_MAX_BPS`. */
export const BOOTSTRAP_BONUS_MAX_BPS = 10_000n;
/** Default first-N-nodes cold-start cap. Mirrors `BOOTSTRAP_NODE_LIMIT`. */
export const BOOTSTRAP_NODE_LIMIT = 10_000n;

/**
 * Reward with the cold-start bonus applied on top of the base [`trafficReward`]. Mirrors
 * `weft_primitives::traffic_reward_with_bootstrap`: the bonus is the last multiplier so
 * the base formula is unchanged; the caller passes `0` when the node is ineligible.
 */
export function trafficRewardWithBootstrap(
  bytes: bigint,
  reputationBps: bigint,
  geoBonusBps: bigint,
  stakingBonusBps: bigint,
  bootstrapBonusBps: bigint,
): bigint {
  const base = trafficReward(bytes, reputationBps, geoBonusBps, stakingBonusBps);
  const bonus =
    BPS +
    (bootstrapBonusBps > BOOTSTRAP_BONUS_MAX_BPS ? BOOTSTRAP_BONUS_MAX_BPS : bootstrapBonusBps);
  const reward = (base * bonus) / BPS;
  return reward > U64_MAX ? U64_MAX : reward;
}

export interface PaymentSplit {
  nodes: bigint;
  burn: bigint;
  treasury: bigint;
}

/** Mirror of `weft_primitives::split_payment` (70/20/10, remainder → treasury). */
export function splitPayment(amount: bigint): PaymentSplit {
  const nodes = (amount * SPLIT_NODES_BPS) / BPS;
  const burn = (amount * SPLIT_BURN_BPS) / BPS;
  const treasury = amount - nodes - burn;
  return { nodes, burn, treasury };
}

/** Default IDO TGE unlock share: 25%. Mirrors `TGE_UNLOCK_BPS`. */
export const TGE_UNLOCK_BPS = 2_500n;

/**
 * Split an IDO allocation into the TGE (immediate) and vesting (linear) portions, the
 * mirror of `weft_primitives::split_tge`. The TGE share rounds down; the vesting share
 * gets the exact remainder.
 */
export function splitTge(allocation: bigint, tgeBps: bigint): { tge: bigint; vesting: bigint } {
  const tge = (allocation * (tgeBps > BPS ? BPS : tgeBps)) / BPS;
  return { tge, vesting: allocation - tge };
}

// ---- merkle (mirror weft-primitives::merkle) ----

function u64le(value: bigint): Uint8Array {
  const out = new Uint8Array(8);
  let v = value;
  for (let i = 0; i < 8; i++) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}

function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const len = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(len);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

/**
 * The epoch-bound, domain-separated reward leaf:
 * `sha256(0x00 ‖ sha256(operator ‖ node_id_le ‖ amount_le ‖ epoch_le))`.
 */
export function hashRewardLeaf(
  epoch: bigint,
  operator: Uint8Array,
  nodeId: bigint,
  amount: bigint,
): Uint8Array {
  if (operator.length !== 32) throw new Error('operator must be 32 bytes');
  const inner = sha256(concatBytes(operator, u64le(nodeId), u64le(amount), u64le(epoch)));
  return sha256(concatBytes(Uint8Array.of(0x00), inner));
}

/**
 * The IDO/TGE allocation leaf (domain `0x02`, distributor-bound), mirror of
 * `weft_primitives::merkle::hash_allocation_leaf`:
 * `sha256(0x02 ‖ sha256(distributor ‖ claimant ‖ amount_le))`.
 */
export function hashAllocationLeaf(
  distributor: Uint8Array,
  claimant: Uint8Array,
  amount: bigint,
): Uint8Array {
  if (distributor.length !== 32 || claimant.length !== 32)
    throw new Error('distributor and claimant must be 32 bytes');
  const inner = sha256(concatBytes(distributor, claimant, u64le(amount)));
  return sha256(concatBytes(Uint8Array.of(0x02), inner));
}

function lte(a: Uint8Array, b: Uint8Array): boolean {
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return a[i] < b[i];
  }
  return true;
}

/** Sorted-pair intermediate hash with the `0x01` domain prefix. */
export function hashPair(a: Uint8Array, b: Uint8Array): Uint8Array {
  const [lo, hi] = lte(a, b) ? [a, b] : [b, a];
  return sha256(concatBytes(Uint8Array.of(0x01), lo, hi));
}

function nextLevel(level: Uint8Array[]): Uint8Array[] {
  const next: Uint8Array[] = [];
  for (let i = 0; i < level.length; i += 2) {
    next.push(i + 1 < level.length ? hashPair(level[i], level[i + 1]) : level[i]);
  }
  return next;
}

/** Build the merkle root from leaves (must be non-empty). */
export function merkleRoot(leaves: Uint8Array[]): Uint8Array {
  if (leaves.length === 0) throw new Error('empty tree');
  let level = leaves;
  while (level.length > 1) level = nextLevel(level);
  return level[0];
}

/** Build the sibling-path proof for the leaf at `index`. */
export function merkleProof(leaves: Uint8Array[], index: number): Uint8Array[] {
  const proof: Uint8Array[] = [];
  let idx = index;
  let level = leaves;
  while (level.length > 1) {
    const sibling = idx % 2 === 0 ? idx + 1 : idx - 1;
    if (sibling < level.length) proof.push(level[sibling]);
    level = nextLevel(level);
    idx = Math.floor(idx / 2);
  }
  return proof;
}

/** Verify a sorted-pair proof against a root (mirror of the on-chain check). */
export function merkleVerify(proof: Uint8Array[], root: Uint8Array, leaf: Uint8Array): boolean {
  let h = leaf;
  for (const p of proof) h = hashPair(h, p);
  return lte(h, root) && lte(root, h); // byte-equal
}

/** Hex-encode bytes (lowercase, no prefix). */
export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

/** Decode a lowercase hex string (no prefix) into bytes. */
export function fromHex(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}
