// Epoch reward construction: verified receipts → per-node byte totals → reward
// amounts (the on-chain `traffic_reward` mirror) → a sorted merkle tree whose
// root is posted on-chain and whose proofs are served to claimants.
//
// Anti-wash gating (the cheap, always-on layer; full Sybil-resistance is M8):
//   • a node earns only if it is registered AND stakes ≥ `minStakeToEarn`
//     (fabrication is then -EV because the dispute path slashes that stake);
//   • per-(operator,node) bytes are capped at `maxBytesPerEpoch` so a single
//     receipt set can't claim implausible volume.

import { getAddressEncoder, type Address } from '@solana/kit';
import { math } from '@weft/sdk';

import { EPOCH_SECONDS } from './epoch';
import { geoBonusBps, EMPTY_GEO_TABLE, type GeoTable } from './geo';
import { selectReceiptsForEpoch, type TrafficReceipt } from './receipts';

const addrEnc = getAddressEncoder();

/** The reward-relevant view of a node (from `NodeState`). */
export interface NodeInfo {
  operator: Address;
  nodeId: bigint;
  endpointHash?: string;
  reputationBps: number;
  geo: number;
  stake: bigint;
  /** Global registration order from `NodeState.sequence` (0 = pre-M8 / unknown). */
  sequence?: bigint;
}

/** The DAO-governed cold-start bonus (from `ProtocolConfig`). */
export interface BootstrapConfig {
  /** Nodes with `sequence <= nodeLimit` are eligible. */
  nodeLimit: bigint;
  /** Bonus applied to the base reward (bps). */
  bonusBps: bigint;
  /** Bonus expires at this unix timestamp (0 = never). */
  endTs: bigint;
}

export interface BuildOptions {
  geoTable?: GeoTable;
  /** Minimum stake (base units) a node must hold to earn. Default 1,000 $WEFT. */
  minStakeToEarn?: bigint;
  /** Per-(operator,node) byte cap per epoch. Default 5 TB. */
  maxBytesPerEpoch?: bigint;
  /** Cold-start bonus (M8). Omit to disable. */
  bootstrap?: BootstrapConfig;
  /** Reward-mint decimals (read from the mint). $WEFT math is 6-decimal; a mint with
   *  more decimals (e.g. a 9-decimal devnet test mint) scales up. Default 6 (mainnet). */
  decimals?: number;
  /** Solvency cap (base units): the reward vault's spendable balance for this epoch. If the
   *  epoch's computed rewards exceed it, every node's reward is scaled down proportionally so the
   *  total never exceeds what the vault can pay — post_epoch's on-chain solvency guard would
   *  otherwise reject the whole epoch and settlement would stall. Omit to disable capping. */
  vaultCap?: bigint;
}

export const DEFAULT_MIN_STAKE_TO_EARN = 1_000n * math.ONE_WEFT;
export const DEFAULT_MAX_BYTES_PER_EPOCH = 5_000_000_000_000n; // 5 TB / 10 min

function nodeShareCap(bytes: bigint): bigint {
  return (math.NODE_REWARD_RATE_PER_GB * bytes) / math.BYTES_PER_GB;
}

/** The cold-start bonus (bps) a node earns this epoch, or 0n if ineligible. */
function bootstrapBonusFor(
  info: NodeInfo,
  cfg: BootstrapConfig | undefined,
  epoch: bigint,
): bigint {
  if (!cfg || cfg.bonusBps === 0n) return 0n;
  const seq = info.sequence ?? 0n;
  if (seq === 0n || seq > cfg.nodeLimit) return 0n;
  // The bonus applies while the epoch's window starts before the governed end.
  if (cfg.endTs !== 0n) {
    const epochStart = epoch * EPOCH_SECONDS;
    if (epochStart >= cfg.endTs) return 0n;
  }
  return cfg.bonusBps;
}

export interface NodeReward {
  operator: Address;
  nodeId: bigint;
  bytes: bigint;
  reward: bigint;
  reputationBps: number;
  geoBonusBps: number;
  stakingBonusBps: number;
  bootstrapBonusBps: number;
}

export interface ByteTotal {
  operator: Address;
  nodeId: bigint;
  bytes: bigint;
}

export interface EpochEntry {
  operator: Address;
  nodeId: bigint;
  amount: bigint;
  leaf: string;
  proof: string[];
}

export interface SkippedNode {
  operator: Address;
  nodeId: bigint;
  bytes: bigint;
  reason: 'unknown-node' | 'below-min-stake' | 'zero-reward';
}

export interface EpochBuild {
  epoch: bigint;
  root: string;
  totalReward: bigint;
  numNodes: number;
  entries: EpochEntry[];
  rewards: NodeReward[];
  skipped: SkippedNode[];
  rejectedReceipts: number;
}

function key(operator: Address, nodeId: bigint): string {
  return `${operator}:${nodeId}`;
}

/**
 * Build one epoch's reward distribution. Pure: it takes the receipt batch and a
 * snapshot of node info, and returns the tree + per-node breakdown. The caller
 * supplies `nodes` (fetched from `NodeState`) and the governance `geoTable`.
 */
export function buildEpoch(
  epoch: bigint,
  receipts: TrafficReceipt[],
  nodes: NodeInfo[],
  opts: BuildOptions = {},
): EpochBuild {
  const maxBytes = opts.maxBytesPerEpoch ?? DEFAULT_MAX_BYTES_PER_EPOCH;

  const { accepted, rejected } = selectReceiptsForEpoch(receipts, epoch);

  // Sum bytes per (operator, node), capped.
  const byteTotals = new Map<string, ByteTotal>();
  for (const r of accepted) {
    const k = key(r.operator, r.nodeId);
    const cur = byteTotals.get(k) ?? { operator: r.operator, nodeId: r.nodeId, bytes: 0n };
    cur.bytes += r.bytes;
    if (cur.bytes > maxBytes) cur.bytes = maxBytes;
    byteTotals.set(k, cur);
  }

  return buildEpochFromByteTotals(epoch, [...byteTotals.values()], nodes, opts, rejected.length);
}

/**
 * Build one epoch from already-trusted byte totals. This is for relay-side metering
 * integrations; `buildEpoch` remains the dual-signed receipt path.
 */
export function buildEpochFromByteTotals(
  epoch: bigint,
  totals: ByteTotal[],
  nodes: NodeInfo[],
  opts: BuildOptions = {},
  rejectedReceipts = 0,
): EpochBuild {
  const geoTable = opts.geoTable ?? EMPTY_GEO_TABLE;
  // $WEFT math is expressed in 6-decimal units; scale to the reward mint's decimals
  // (1× on mainnet's 6-decimal token, 1000× on a 9-decimal devnet test mint).
  const scale = math.baseUnitScale(opts.decimals ?? 6);
  const stakingThreshold = math.STAKING_BONUS_THRESHOLD * scale;
  const minStake = opts.minStakeToEarn ?? DEFAULT_MIN_STAKE_TO_EARN * scale;

  const nodeByKey = new Map<string, NodeInfo>();
  for (const n of nodes) nodeByKey.set(key(n.operator, n.nodeId), n);

  const rewards: NodeReward[] = [];
  const skipped: SkippedNode[] = [];
  for (const { operator, nodeId, bytes } of totals) {
    const info = nodeByKey.get(key(operator, nodeId));
    if (!info) {
      skipped.push({ operator, nodeId, bytes, reason: 'unknown-node' });
      continue;
    }
    if (info.stake < minStake) {
      skipped.push({ operator, nodeId, bytes, reason: 'below-min-stake' });
      continue;
    }
    const geoBonus = geoBonusBps(geoTable, info.geo);
    // stake is in the mint's base units; compare against the decimals-scaled threshold.
    const stakingBonus = info.stake >= stakingThreshold ? Number(math.STAKING_BONUS_BPS) : 0;
    // Cold-start bonus (M8): a node within the early-adopter sequence limit, while the
    // governed window is open, earns an extra multiplier on top of the base reward.
    const bootstrapBonus = bootstrapBonusFor(info, opts.bootstrap, epoch);
    const uncappedReward = math.trafficRewardWithBootstrap(
      bytes,
      BigInt(info.reputationBps),
      BigInt(geoBonus),
      BigInt(stakingBonus),
      bootstrapBonus,
    );
    const cap = nodeShareCap(bytes);
    // Cap in 6-decimal units, then scale the final amount to the mint's base units.
    const reward = (uncappedReward > cap ? cap : uncappedReward) * scale;
    if (reward === 0n) {
      skipped.push({ operator, nodeId, bytes, reason: 'zero-reward' });
      continue;
    }
    rewards.push({
      operator,
      nodeId,
      bytes,
      reward,
      reputationBps: info.reputationBps,
      geoBonusBps: geoBonus,
      stakingBonusBps: stakingBonus,
      bootstrapBonusBps: Number(bootstrapBonus),
    });
  }

  // Vault-solvency cap: the reward vault only holds fees actually collected (70% of what users
  // paid). If the epoch's rewards exceed it — e.g. a node served more bytes than the user's escrow
  // covered — scale every node's reward down proportionally so the total never exceeds the vault.
  // Otherwise post_epoch's on-chain solvency guard rejects the whole epoch and nothing ever settles.
  if (opts.vaultCap != null && opts.vaultCap >= 0n) {
    const total = rewards.reduce((s, r) => s + r.reward, 0n);
    if (total > opts.vaultCap) {
      for (const r of rewards) r.reward = (r.reward * opts.vaultCap) / total;
      for (let i = rewards.length - 1; i >= 0; i--) {
        if (rewards[i].reward === 0n) {
          skipped.push({
            operator: rewards[i].operator,
            nodeId: rewards[i].nodeId,
            bytes: rewards[i].bytes,
            reason: 'zero-reward',
          });
          rewards.splice(i, 1);
        }
      }
    }
  }

  // Deterministic order: by operator address, then node id.
  rewards.sort((a, b) =>
    a.operator === b.operator
      ? a.nodeId < b.nodeId
        ? -1
        : a.nodeId > b.nodeId
          ? 1
          : 0
      : a.operator < b.operator
        ? -1
        : 1,
  );

  const leaves = rewards.map((r) =>
    math.hashRewardLeaf(epoch, addrEnc.encode(r.operator) as Uint8Array, r.nodeId, r.reward),
  );

  const entries: EpochEntry[] = rewards.map((r, i) => ({
    operator: r.operator,
    nodeId: r.nodeId,
    amount: r.reward,
    leaf: math.toHex(leaves[i]),
    proof: leaves.length > 0 ? math.merkleProof(leaves, i).map(math.toHex) : [],
  }));

  const totalReward = rewards.reduce((s, r) => s + r.reward, 0n);
  const root = leaves.length > 0 ? math.toHex(math.merkleRoot(leaves)) : '';

  return {
    epoch,
    root,
    totalReward,
    numNodes: rewards.length,
    entries,
    rewards,
    skipped,
    rejectedReceipts,
  };
}
