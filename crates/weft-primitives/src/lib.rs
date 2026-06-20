//! Shared Weft protocol primitives.
//!
//! Single source of truth for tokenomics constants and the reward / payment-split
//! math defined in `SPEC.md`. All arithmetic is integer-only (no floating point)
//! and `no_std` so the exact same code runs on-chain (SBF programs), in the
//! off-chain settlement aggregator, and in tests — guaranteeing the four
//! components never disagree on how much a node is owed.
//!
//! Ratios are expressed in **basis points** (bps): `10_000 bps = 100% = 1.0x`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod merkle;

/// Basis-point denominator. `10_000 bps == 100%`.
pub const BPS: u32 = 10_000;

// ---------------------------------------------------------------------------
// $WEFT token
// ---------------------------------------------------------------------------

/// Token decimals. 9 mirrors SOL and gives sub-micro-token granularity for
/// per-gigabyte micropayments.
pub const DECIMALS: u8 = 9;

/// Base units in one whole $WEFT (`10^DECIMALS`).
pub const ONE_WEFT: u64 = 1_000_000_000;

/// Maximum total supply in whole tokens (`SPEC.md`: 1,000,000,000).
pub const TOTAL_SUPPLY: u64 = 1_000_000_000;

/// Maximum total supply expressed in base units (`10^18`, fits in `u64`).
pub const TOTAL_SUPPLY_BASE_UNITS: u64 = TOTAL_SUPPLY * ONE_WEFT;

/// Token distribution buckets, in basis points of [`TOTAL_SUPPLY`] (`SPEC.md`).
pub mod allocation_bps {
    /// Node operator rewards — emission pool for network operation.
    pub const NODE_OPERATORS: u32 = 4_000;
    /// Public round / IDO — 25% at TGE, remainder over 12 months.
    pub const PUBLIC_IDO: u32 = 1_500;
    /// Team and advisors — 12-month cliff, 36-month vesting.
    pub const TEAM: u32 = 1_500;
    /// Ecosystem and grants — 36 months, by DAO decision.
    pub const ECOSYSTEM: u32 = 1_500;
    /// Treasury / reserve — managed by multisig.
    pub const TREASURY: u32 = 1_000;
    /// Marketing and partnerships — 24 months.
    pub const MARKETING: u32 = 500;
}

/// Amount of base units for an allocation expressed in basis points of supply.
pub const fn allocation_amount(bps: u32) -> u64 {
    ((TOTAL_SUPPLY_BASE_UNITS as u128 * bps as u128) / BPS as u128) as u64
}

// ---------------------------------------------------------------------------
// Node reward formula (`SPEC.md` > Node Economics)
// ---------------------------------------------------------------------------

/// Base emission rate: 0.1 $WEFT per 1 GB of transferred traffic.
pub const BASE_RATE_PER_GB: u64 = ONE_WEFT / 10;

/// Bytes in one billable gigabyte (decimal GB).
pub const BYTES_PER_GB: u64 = 1_000_000_000;

/// Reputation multiplier floor: 0.5x.
pub const REPUTATION_MIN_BPS: u32 = 5_000;
/// Reputation multiplier ceiling: 2.0x.
pub const REPUTATION_MAX_BPS: u32 = 20_000;

/// Geographic-demand bonus ceiling: up to +50%.
pub const GEO_BONUS_MAX_BPS: u32 = 5_000;

/// Staking bonus: +20% reward for nodes staking at or above the threshold.
pub const STAKING_BONUS_BPS: u32 = 2_000;
/// Stake (base units) required to earn the staking bonus: 10,000 $WEFT.
pub const STAKING_BONUS_THRESHOLD: u64 = 10_000 * ONE_WEFT;

/// Settlement epoch length: rewards settle every 10 minutes.
pub const EPOCH_SECONDS: u64 = 600;

/// Clamp a reputation multiplier into the allowed `[0.5x, 2.0x]` range.
pub fn clamp_reputation_bps(reputation_bps: u32) -> u32 {
    reputation_bps.clamp(REPUTATION_MIN_BPS, REPUTATION_MAX_BPS)
}

/// Clamp a geo-demand bonus into the allowed `[0, +50%]` range.
pub fn clamp_geo_bonus_bps(geo_bonus_bps: u32) -> u32 {
    geo_bonus_bps.min(GEO_BONUS_MAX_BPS)
}

/// Staking bonus (bps) a node earns given its staked balance in base units.
pub fn staking_bonus_for_stake(staked_base_units: u64) -> u32 {
    if staked_base_units >= STAKING_BONUS_THRESHOLD {
        STAKING_BONUS_BPS
    } else {
        0
    }
}

/// Reward (in $WEFT base units) for `bytes` of relayed traffic.
///
/// `reward = 0.1 WEFT/GB · (bytes/GB) · reputation · (1 + geo_bonus) · (1 + staking_bonus)`
///
/// All multiplier inputs are clamped to their spec-defined ranges so a
/// malformed on-chain or off-chain value can never inflate a payout. The
/// computation runs in `u128` and saturates at `u64::MAX`.
pub fn traffic_reward(
    bytes: u64,
    reputation_bps: u32,
    geo_bonus_bps: u32,
    staking_bonus_bps: u32,
) -> u64 {
    let base = (BASE_RATE_PER_GB as u128 * bytes as u128) / BYTES_PER_GB as u128;
    let reputation = clamp_reputation_bps(reputation_bps) as u128;
    let geo = (BPS + clamp_geo_bonus_bps(geo_bonus_bps)) as u128;
    let staking = (BPS + staking_bonus_bps.min(STAKING_BONUS_BPS)) as u128;
    let denom = BPS as u128;

    let reward = base * reputation / denom * geo / denom * staking / denom;
    if reward > u64::MAX as u128 {
        u64::MAX
    } else {
        reward as u64
    }
}

// ---------------------------------------------------------------------------
// Traffic receipts (M6 metering ↔ M4 settlement)
// ---------------------------------------------------------------------------

/// Byte length of the canonical dual-signed traffic-receipt signing preimage.
pub const RECEIPT_CORE_LEN: usize = 104;

/// Canonical signing preimage for a dual-signed traffic receipt, byte-identical to
/// the off-chain aggregator's `encodeReceiptCore` (`services/aggregator/src/receipts.ts`)
/// so a receipt minted by the Rust data plane (M6) verifies in the TS aggregator (M4)
/// and feeds the on-chain reward merkle. Layout (no padding, all little-endian):
/// `client(32) ‖ operator(32) ‖ node_id ‖ bytes ‖ window_start ‖ window_end ‖ nonce`.
/// The client signs `client_sig` and the relay/operator signs `relay_sig`, both over
/// this preimage.
pub fn encode_receipt_core(
    client: &[u8; 32],
    operator: &[u8; 32],
    node_id: u64,
    bytes: u64,
    window_start: u64,
    window_end: u64,
    nonce: u64,
) -> [u8; RECEIPT_CORE_LEN] {
    let mut out = [0u8; RECEIPT_CORE_LEN];
    out[0..32].copy_from_slice(client);
    out[32..64].copy_from_slice(operator);
    out[64..72].copy_from_slice(&node_id.to_le_bytes());
    out[72..80].copy_from_slice(&bytes.to_le_bytes());
    out[80..88].copy_from_slice(&window_start.to_le_bytes());
    out[88..96].copy_from_slice(&window_end.to_le_bytes());
    out[96..104].copy_from_slice(&nonce.to_le_bytes());
    out
}

/// A receipt window belongs to `epoch` iff it is non-empty and lies wholly inside the
/// epoch's `[epoch·EPOCH_SECONDS, (epoch+1)·EPOCH_SECONDS)` range. Mirrors the
/// aggregator's `windowInEpoch`.
pub fn receipt_window_in_epoch(epoch: u64, window_start: u64, window_end: u64) -> bool {
    if window_end <= window_start {
        return false;
    }
    let start = epoch.saturating_mul(EPOCH_SECONDS);
    let end = epoch.saturating_add(1).saturating_mul(EPOCH_SECONDS);
    window_start >= start && window_end <= end
}

// ---------------------------------------------------------------------------
// User traffic-payment split (`SPEC.md` > Burn Mechanism)
// ---------------------------------------------------------------------------

/// Share of a user traffic payment routed to relay nodes.
pub const SPLIT_NODES_BPS: u32 = 7_000;
/// Share of a user traffic payment that is burned.
pub const SPLIT_BURN_BPS: u32 = 2_000;
/// Share of a user traffic payment sent to the treasury.
pub const SPLIT_TREASURY_BPS: u32 = 1_000;

/// Destination amounts when a user traffic payment of `amount` base units is split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaymentSplit {
    /// 70% — paid to relay nodes.
    pub nodes: u64,
    /// 20% — burned.
    pub burn: u64,
    /// 10% — treasury.
    pub treasury: u64,
}

/// Split a user traffic payment 70/20/10 (nodes/burn/treasury).
///
/// Node and burn shares round down; the treasury receives the exact remainder
/// so the parts always sum back to `amount` with zero leaked lamports.
pub fn split_payment(amount: u64) -> PaymentSplit {
    split_payment_bps(amount, SPLIT_NODES_BPS, SPLIT_BURN_BPS)
}

/// Split a user traffic payment using DAO-governed node/burn shares (the rest goes
/// to the treasury). Identical rounding to [`split_payment`] — node and burn shares
/// round down, the treasury gets the exact remainder. The on-chain `pay_traffic`
/// instruction reads `nodes_bps`/`burn_bps` from the governed `ProtocolConfig`, so
/// the split is adjustable by governance without a program upgrade. Callers must
/// ensure `nodes_bps + burn_bps <= BPS` (validated where the config is set).
pub fn split_payment_bps(amount: u64, nodes_bps: u32, burn_bps: u32) -> PaymentSplit {
    let nodes = (amount as u128 * nodes_bps as u128 / BPS as u128) as u64;
    let burn = (amount as u128 * burn_bps as u128 / BPS as u128) as u64;
    let treasury = amount - nodes - burn;
    PaymentSplit {
        nodes,
        burn,
        treasury,
    }
}

// ---------------------------------------------------------------------------
// Token vesting (`SPEC.md` > Token Distribution)
// ---------------------------------------------------------------------------

/// Seconds in one vesting month (30 days). Schedule durations are whole months
/// of this length so the on-chain program and the genesis script agree exactly.
pub const MONTH_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Cliff and total durations (seconds) for each genesis vesting schedule
/// (`SPEC.md` token distribution table). Tokens unlock linearly from `start_ts`
/// to `start_ts + duration`; nothing is claimable until `start_ts + cliff`, at
/// which point the elapsed share unlocks as a lump.
pub mod vesting {
    use super::MONTH_SECONDS;

    /// Team & advisors: 12-month cliff, 36-month total vesting (revocable).
    pub const TEAM_CLIFF: i64 = 12 * MONTH_SECONDS;
    /// See [`TEAM_CLIFF`].
    pub const TEAM_DURATION: i64 = 36 * MONTH_SECONDS;

    /// Public/IDO linear tranche (the 75% not released at TGE): 12 months, no cliff.
    pub const IDO_LINEAR_CLIFF: i64 = 0;
    /// See [`IDO_LINEAR_CLIFF`].
    pub const IDO_LINEAR_DURATION: i64 = 12 * MONTH_SECONDS;

    /// Ecosystem & grants: 36 months, no cliff.
    pub const ECOSYSTEM_CLIFF: i64 = 0;
    /// See [`ECOSYSTEM_CLIFF`].
    pub const ECOSYSTEM_DURATION: i64 = 36 * MONTH_SECONDS;

    /// Marketing & partnerships: 24 months, no cliff.
    pub const MARKETING_CLIFF: i64 = 0;
    /// See [`MARKETING_CLIFF`].
    pub const MARKETING_DURATION: i64 = 24 * MONTH_SECONDS;
}

/// Cumulative amount (base units) vested by `now` for a cliff + linear schedule.
///
/// `cliff_unlock` is an optional lump available at `start_ts` (a TGE portion);
/// the remaining `total - cliff_unlock` vests linearly across `duration`. Before
/// `start_ts + cliff_duration` only the lump is available; at or after
/// `start_ts + duration` the full `total` is returned exactly (no rounding dust).
///
/// Integer-only, saturating, and monotonically non-decreasing in `now`, so
/// subtracting an already-released amount yields a never-negative claimable value.
pub fn vested_amount(
    total: u64,
    cliff_unlock: u64,
    start_ts: i64,
    cliff_duration: i64,
    duration: i64,
    now: i64,
) -> u64 {
    let lump = cliff_unlock.min(total);
    if now <= start_ts {
        return lump;
    }
    let elapsed = now as i128 - start_ts as i128;
    if elapsed < cliff_duration as i128 {
        return lump;
    }
    if duration <= 0 || elapsed >= duration as i128 {
        return total;
    }
    let linear_pool = total.saturating_sub(cliff_unlock) as u128;
    let linear_vested = (linear_pool * elapsed as u128 / duration as u128) as u64;
    lump.saturating_add(linear_vested).min(total)
}

// ---------------------------------------------------------------------------
// Node registry (`SPEC.md` > Node registry) — M2
// ---------------------------------------------------------------------------

/// Bits of geohash stored on-chain: 6 base-32 chars x 5 bits ~= 1.2 km cells.
pub const GEO_BITS: u32 = 30;
/// Maximum valid packed geo value (low [`GEO_BITS`] bits set).
pub const GEO_MAX: u32 = (1 << GEO_BITS) - 1;

/// Whether a packed geo value fits the on-chain geohash field.
pub fn geo_is_valid(geo: u32) -> bool {
    geo <= GEO_MAX
}

/// Top `chars` geohash characters of a packed geo (region prefix), for off-chain
/// demand-coefficient bucketing. `chars` is clamped to `0..=6`.
pub fn geo_region_prefix(geo: u32, chars: u8) -> u32 {
    let keep = (chars.min(6) as u32) * 5;
    let geo = geo & GEO_MAX;
    if keep >= GEO_BITS {
        geo
    } else {
        geo >> (GEO_BITS - keep)
    }
}

/// Node capability bitflags stored in `NodeState.capabilities`.
pub mod capability {
    /// Speaks the WireGuard data plane.
    pub const WIREGUARD: u32 = 1 << 0;
    /// Relays (intermediate hop) traffic.
    pub const RELAY: u32 = 1 << 1;
    /// Acts as an exit node.
    pub const EXIT: u32 = 1 << 2;
    /// Supports IPv6.
    pub const IPV6: u32 = 1 << 3;
    /// Mobile device node.
    pub const MOBILE: u32 = 1 << 4;
    /// Router / OpenWRT node.
    pub const ROUTER: u32 = 1 << 5;
}

/// All recognized capability bits.
pub const CAPABILITIES_MASK: u32 = capability::WIREGUARD
    | capability::RELAY
    | capability::EXIT
    | capability::IPV6
    | capability::MOBILE
    | capability::ROUTER;

/// A capability set is valid if it sets at least one known bit and no unknown bits.
pub fn capabilities_valid(capabilities: u32) -> bool {
    capabilities != 0 && (capabilities & !CAPABILITIES_MASK) == 0
}

/// Maximum self-reported availability percentage.
pub const MAX_AVAILABILITY: u8 = 100;

/// Whether a self-reported availability value is in range.
pub fn availability_is_valid(availability: u8) -> bool {
    availability <= MAX_AVAILABILITY
}

// ---------------------------------------------------------------------------
// Reputation scoring (`SPEC.md` > Reputation) — M3
// ---------------------------------------------------------------------------

/// Reputation signal weights (bps of [`BPS`]; sum to `BPS`).
pub const REPUTATION_W_UPTIME_BPS: u32 = 5_000;
/// See [`REPUTATION_W_UPTIME_BPS`].
pub const REPUTATION_W_SPEED_BPS: u32 = 3_000;
/// See [`REPUTATION_W_UPTIME_BPS`].
pub const REPUTATION_W_REVIEW_BPS: u32 = 2_000;

/// EMA weight of a fresh sample (0.2). Higher = less smoothing.
pub const REPUTATION_EMA_ALPHA_BPS: u32 = 2_000;

/// Quality score that maps to a neutral 1.0x multiplier. New/idle nodes sit here.
pub const REPUTATION_BASELINE_QUALITY_BPS: u32 = 3_333;

/// Fraction pulled toward baseline per elapsed decay period (0.1).
pub const REPUTATION_DECAY_PER_PERIOD_BPS: u32 = 1_000;
/// Length of one staleness decay period (seconds).
pub const REPUTATION_DECAY_PERIOD_SECONDS: i64 = 3_600;
/// Periods after which a stale score is treated as fully decayed to baseline.
const REPUTATION_DECAY_MAX_PERIODS: i64 = 64;

/// Weighted blend of the three signals (each clamped to `0..=BPS`) into a quality
/// score in `0..=BPS`. `u128` intermediate, divide last.
pub fn reputation_quality_bps(uptime_bps: u32, speed_bps: u32, review_bps: u32) -> u32 {
    let u = uptime_bps.min(BPS) as u128;
    let s = speed_bps.min(BPS) as u128;
    let r = review_bps.min(BPS) as u128;
    let q = (u * REPUTATION_W_UPTIME_BPS as u128
        + s * REPUTATION_W_SPEED_BPS as u128
        + r * REPUTATION_W_REVIEW_BPS as u128)
        / BPS as u128;
    q as u32
}

/// Map a quality score (`0..=BPS`) linearly onto `[REPUTATION_MIN_BPS,
/// REPUTATION_MAX_BPS]` (rounded), so `0 -> 0.5x`, baseline `-> 1.0x`, `BPS -> 2.0x`.
pub fn reputation_multiplier_bps(quality_bps: u32) -> u32 {
    let q = quality_bps.min(BPS) as u128;
    let span = (REPUTATION_MAX_BPS - REPUTATION_MIN_BPS) as u128;
    let scaled = (q * span + (BPS as u128 / 2)) / BPS as u128;
    REPUTATION_MIN_BPS + scaled as u32
}

/// EMA update of a stored quality with a fresh sample:
/// `new = (alpha*sample + (BPS-alpha)*old) / BPS`. Result lies between old and sample.
pub fn reputation_ema(old_quality_bps: u32, sample_quality_bps: u32, alpha_bps: u32) -> u32 {
    let old = old_quality_bps.min(BPS) as u128;
    let sample = sample_quality_bps.min(BPS) as u128;
    let alpha = alpha_bps.min(BPS) as u128;
    let new = (alpha * sample + (BPS as u128 - alpha) * old) / BPS as u128;
    new as u32
}

/// Decay a stored quality toward [`REPUTATION_BASELINE_QUALITY_BPS`] by whole
/// elapsed decay periods (compounded 10%/period), before a fresh sample is blended.
/// Monotonic toward baseline, never overshoots; fixed point at baseline.
pub fn reputation_decayed_quality_bps(old_quality_bps: u32, now: i64, last_update: i64) -> u32 {
    let old = old_quality_bps.min(BPS);
    if now <= last_update {
        return old;
    }
    let periods = (now - last_update) / REPUTATION_DECAY_PERIOD_SECONDS;
    if periods <= 0 {
        return old;
    }
    if periods >= REPUTATION_DECAY_MAX_PERIODS {
        return REPUTATION_BASELINE_QUALITY_BPS;
    }
    let base = REPUTATION_BASELINE_QUALITY_BPS as i128;
    let decay = REPUTATION_DECAY_PER_PERIOD_BPS as i128;
    let mut q = old as i128;
    for _ in 0..periods {
        q += (base - q) * decay / BPS as i128;
    }
    q as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_supply_is_1e18_base_units() {
        assert_eq!(TOTAL_SUPPLY_BASE_UNITS, 1_000_000_000_000_000_000);
        assert_eq!(ONE_WEFT, 10u64.pow(DECIMALS as u32));
    }

    #[test]
    fn allocations_match_spec_table() {
        assert_eq!(
            allocation_amount(allocation_bps::NODE_OPERATORS),
            400_000_000 * ONE_WEFT
        );
        assert_eq!(
            allocation_amount(allocation_bps::PUBLIC_IDO),
            150_000_000 * ONE_WEFT
        );
        assert_eq!(
            allocation_amount(allocation_bps::TEAM),
            150_000_000 * ONE_WEFT
        );
        assert_eq!(
            allocation_amount(allocation_bps::ECOSYSTEM),
            150_000_000 * ONE_WEFT
        );
        assert_eq!(
            allocation_amount(allocation_bps::TREASURY),
            100_000_000 * ONE_WEFT
        );
        assert_eq!(
            allocation_amount(allocation_bps::MARKETING),
            50_000_000 * ONE_WEFT
        );
    }

    #[test]
    fn allocations_sum_to_total_supply() {
        let total = allocation_amount(allocation_bps::NODE_OPERATORS)
            + allocation_amount(allocation_bps::PUBLIC_IDO)
            + allocation_amount(allocation_bps::TEAM)
            + allocation_amount(allocation_bps::ECOSYSTEM)
            + allocation_amount(allocation_bps::TREASURY)
            + allocation_amount(allocation_bps::MARKETING);
        assert_eq!(total, TOTAL_SUPPLY_BASE_UNITS);

        let bps_sum = allocation_bps::NODE_OPERATORS
            + allocation_bps::PUBLIC_IDO
            + allocation_bps::TEAM
            + allocation_bps::ECOSYSTEM
            + allocation_bps::TREASURY
            + allocation_bps::MARKETING;
        assert_eq!(bps_sum, BPS);
    }

    #[test]
    fn neutral_reward_is_base_rate_per_gb() {
        // 1 GB, reputation 1.0x, no geo bonus, no staking bonus.
        let r = traffic_reward(BYTES_PER_GB, BPS, 0, 0);
        assert_eq!(r, BASE_RATE_PER_GB);
        assert_eq!(r, 100_000_000); // 0.1 WEFT
    }

    #[test]
    fn maxed_out_reward() {
        // 1 GB, 2.0x reputation, +50% geo, +20% staking = 0.1 * 2.0 * 1.5 * 1.2 = 0.36 WEFT.
        let r = traffic_reward(
            BYTES_PER_GB,
            REPUTATION_MAX_BPS,
            GEO_BONUS_MAX_BPS,
            STAKING_BONUS_BPS,
        );
        assert_eq!(r, 360_000_000);
    }

    #[test]
    fn minimum_reputation_halves_reward() {
        let r = traffic_reward(BYTES_PER_GB, REPUTATION_MIN_BPS, 0, 0);
        assert_eq!(r, 50_000_000); // 0.05 WEFT
    }

    #[test]
    fn multiplier_inputs_are_clamped() {
        // Reputation above ceiling clamps to 2.0x, below floor clamps to 0.5x.
        assert_eq!(
            traffic_reward(BYTES_PER_GB, 100_000, 0, 0),
            traffic_reward(BYTES_PER_GB, REPUTATION_MAX_BPS, 0, 0)
        );
        assert_eq!(
            traffic_reward(BYTES_PER_GB, 1, 0, 0),
            traffic_reward(BYTES_PER_GB, REPUTATION_MIN_BPS, 0, 0)
        );
        // Geo bonus above +50% clamps to +50%.
        assert_eq!(
            traffic_reward(BYTES_PER_GB, BPS, 100_000, 0),
            traffic_reward(BYTES_PER_GB, BPS, GEO_BONUS_MAX_BPS, 0)
        );
        // Staking bonus above +20% clamps to +20%.
        assert_eq!(
            traffic_reward(BYTES_PER_GB, BPS, 0, 100_000),
            traffic_reward(BYTES_PER_GB, BPS, 0, STAKING_BONUS_BPS)
        );
    }

    #[test]
    fn staking_bonus_threshold_boundary() {
        assert_eq!(staking_bonus_for_stake(STAKING_BONUS_THRESHOLD - 1), 0);
        assert_eq!(
            staking_bonus_for_stake(STAKING_BONUS_THRESHOLD),
            STAKING_BONUS_BPS
        );
        assert_eq!(staking_bonus_for_stake(u64::MAX), STAKING_BONUS_BPS);
    }

    #[test]
    fn huge_traffic_does_not_panic_and_saturates() {
        // Must not overflow-panic even at the extreme.
        let r = traffic_reward(
            u64::MAX,
            REPUTATION_MAX_BPS,
            GEO_BONUS_MAX_BPS,
            STAKING_BONUS_BPS,
        );
        assert!(r > 0);
    }

    #[test]
    fn split_is_exact_70_20_10() {
        let s = split_payment(1_000);
        assert_eq!(
            s,
            PaymentSplit {
                nodes: 700,
                burn: 200,
                treasury: 100
            }
        );
    }

    #[test]
    fn split_remainder_goes_to_treasury_and_conserves_total() {
        for amount in [0u64, 1, 7, 9_999, 12_345, u64::MAX] {
            let s = split_payment(amount);
            assert_eq!(
                s.nodes as u128 + s.burn as u128 + s.treasury as u128,
                amount as u128,
                "split of {amount} must conserve total"
            );
        }
        // Rounding example: shares round down, treasury absorbs the remainder.
        let s = split_payment(9_999);
        assert_eq!(s.nodes, 6_999);
        assert_eq!(s.burn, 1_999);
        assert_eq!(s.treasury, 1_001);
    }

    #[test]
    fn receipt_core_layout_is_canonical() {
        let client = [1u8; 32];
        let operator = [2u8; 32];
        let core = encode_receipt_core(&client, &operator, 7, 123_456, 600, 601, 9);
        assert_eq!(core.len(), RECEIPT_CORE_LEN);
        assert_eq!(&core[0..32], &client);
        assert_eq!(&core[32..64], &operator);
        assert_eq!(&core[64..72], &7u64.to_le_bytes());
        assert_eq!(&core[72..80], &123_456u64.to_le_bytes());
        assert_eq!(&core[80..88], &600u64.to_le_bytes());
        assert_eq!(&core[88..96], &601u64.to_le_bytes());
        assert_eq!(&core[96..104], &9u64.to_le_bytes());
    }

    #[test]
    fn receipt_window_epoch_bounds() {
        // epoch 1 spans [600, 1200)
        assert!(receipt_window_in_epoch(1, 600, 1200));
        assert!(receipt_window_in_epoch(1, 700, 800));
        assert!(!receipt_window_in_epoch(1, 599, 800)); // starts before
        assert!(!receipt_window_in_epoch(1, 700, 1201)); // ends after
        assert!(!receipt_window_in_epoch(1, 800, 800)); // empty
        assert!(!receipt_window_in_epoch(0, 600, 1200)); // wrong epoch
    }

    #[test]
    fn split_payment_bps_matches_constant_split_at_defaults() {
        // The governed split with the default bps must be byte-identical to the
        // compile-time `split_payment`, so the M5 migration changes nothing at par.
        for amount in [0u64, 1, 7, 9_999, 12_345, 1_000_000, u64::MAX] {
            assert_eq!(
                split_payment_bps(amount, SPLIT_NODES_BPS, SPLIT_BURN_BPS),
                split_payment(amount)
            );
        }
        // A DAO-adjusted split (60/30/10) still conserves the total.
        let s = split_payment_bps(1_000_000, 6_000, 3_000);
        assert_eq!(s.nodes, 600_000);
        assert_eq!(s.burn, 300_000);
        assert_eq!(s.treasury, 100_000);
    }

    #[test]
    fn vesting_before_start_returns_only_lump() {
        assert_eq!(vested_amount(100, 10, 1_000, 0, 100, 500), 10);
        assert_eq!(vested_amount(100, 0, 1_000, 0, 100, 500), 0);
    }

    #[test]
    fn vesting_no_cliff_linear_midpoint_and_full() {
        assert_eq!(vested_amount(100, 0, 0, 0, 100, 50), 50);
        assert_eq!(vested_amount(100, 0, 0, 0, 100, 100), 100);
        assert_eq!(vested_amount(100, 0, 0, 0, 100, 250), 100);
    }

    #[test]
    fn vesting_cliff_lump_unlocks_at_boundary() {
        // 36 units over 36 "months", 12-month cliff: nothing until month 12,
        // then the elapsed 1/3 unlocks as a lump and linear continues.
        assert_eq!(vested_amount(36, 0, 0, 12, 36, 11), 0);
        assert_eq!(vested_amount(36, 0, 0, 12, 36, 12), 12);
        assert_eq!(vested_amount(36, 0, 0, 12, 36, 24), 24);
        assert_eq!(vested_amount(36, 0, 0, 12, 36, 36), 36);
    }

    #[test]
    fn vesting_with_tge_lump_then_linear() {
        // 100 total, 25 at TGE, remaining 75 linear over 12.
        assert_eq!(vested_amount(100, 25, 0, 0, 12, 0), 25);
        assert_eq!(vested_amount(100, 25, 0, 0, 12, 6), 25 + 75 * 6 / 12); // 62
        assert_eq!(vested_amount(100, 25, 0, 0, 12, 12), 100);
    }

    #[test]
    fn vesting_full_vest_is_exact_and_rounds_down_midway() {
        assert_eq!(vested_amount(100, 0, 0, 0, 7, 3), 100 * 3 / 7); // 42, rounds down
        assert_eq!(vested_amount(100, 0, 0, 0, 7, 7), 100); // exact, no dust stranded
    }

    #[test]
    fn vesting_zero_duration_returns_total_without_panic() {
        assert_eq!(vested_amount(100, 0, 0, 0, 0, 1), 100);
    }

    #[test]
    fn vesting_is_monotonic_non_decreasing() {
        let mut prev = 0;
        for now in -5..150 {
            let v = vested_amount(100, 10, 0, 12, 100, now);
            assert!(v >= prev, "vested decreased at now={now}: {v} < {prev}");
            assert!(v <= 100);
            prev = v;
        }
    }

    #[test]
    fn vesting_saturates_on_huge_amounts() {
        let v = vested_amount(u64::MAX, 0, 0, 0, 1_000, 500);
        assert!(v > 0 && v < u64::MAX);
    }

    #[test]
    fn vesting_team_schedule_matches_spec_at_cliff() {
        // Team: 150M over 36 months, 12-month cliff → exactly 1/3 (50M) at the cliff.
        let total = allocation_amount(allocation_bps::TEAM);
        let at_cliff = vested_amount(
            total,
            0,
            0,
            vesting::TEAM_CLIFF,
            vesting::TEAM_DURATION,
            vesting::TEAM_CLIFF,
        );
        assert_eq!(at_cliff, 50_000_000 * ONE_WEFT);
        // Fully vested at month 36.
        let at_end = vested_amount(
            total,
            0,
            0,
            vesting::TEAM_CLIFF,
            vesting::TEAM_DURATION,
            vesting::TEAM_DURATION,
        );
        assert_eq!(at_end, total);
    }

    #[test]
    fn geo_validation_and_region_prefix() {
        assert!(geo_is_valid(0));
        assert!(geo_is_valid(GEO_MAX));
        assert!(!geo_is_valid(GEO_MAX + 1));

        let geo = 0b11111_00000_11111_00000_11111_00000;
        assert_eq!(geo_region_prefix(geo, 1), 0b11111);
        assert_eq!(geo_region_prefix(geo, 2), 0b11111_00000);
        assert_eq!(geo_region_prefix(geo, 6), geo & GEO_MAX);
        assert_eq!(geo_region_prefix(geo, 0), 0);
    }

    #[test]
    fn capability_validation() {
        assert_eq!(CAPABILITIES_MASK, (1 << 6) - 1);
        assert!(capabilities_valid(capability::WIREGUARD));
        assert!(capabilities_valid(capability::WIREGUARD | capability::EXIT));
        assert!(!capabilities_valid(0)); // must set at least one
        assert!(!capabilities_valid(1 << 30)); // unknown bit
    }

    #[test]
    fn availability_bounds() {
        assert!(availability_is_valid(0));
        assert!(availability_is_valid(MAX_AVAILABILITY));
        assert!(!availability_is_valid(101));
    }

    #[test]
    fn reputation_weights_sum_to_bps() {
        assert_eq!(
            REPUTATION_W_UPTIME_BPS + REPUTATION_W_SPEED_BPS + REPUTATION_W_REVIEW_BPS,
            BPS
        );
    }

    #[test]
    fn reputation_quality_blend() {
        assert_eq!(reputation_quality_bps(BPS, BPS, BPS), BPS); // all max
        assert_eq!(reputation_quality_bps(0, 0, 0), 0);
        // 0.5*10000 + 0.3*0 + 0.2*0 = 5000
        assert_eq!(reputation_quality_bps(BPS, 0, 0), 5_000);
        // clamps out-of-range inputs
        assert_eq!(reputation_quality_bps(u32::MAX, u32::MAX, u32::MAX), BPS);
    }

    #[test]
    fn reputation_multiplier_endpoints_and_baseline() {
        assert_eq!(reputation_multiplier_bps(0), REPUTATION_MIN_BPS); // 0.5x
        assert_eq!(reputation_multiplier_bps(BPS), REPUTATION_MAX_BPS); // 2.0x
        assert_eq!(
            reputation_multiplier_bps(REPUTATION_BASELINE_QUALITY_BPS),
            BPS
        ); // 1.0x
        assert_eq!(reputation_multiplier_bps(u32::MAX), REPUTATION_MAX_BPS); // clamped
    }

    #[test]
    fn reputation_ema_is_bounded_and_converges() {
        // ema(x, x, a) == x
        assert_eq!(
            reputation_ema(4_000, 4_000, REPUTATION_EMA_ALPHA_BPS),
            4_000
        );
        // result lies between old and sample
        let e = reputation_ema(2_000, 8_000, REPUTATION_EMA_ALPHA_BPS);
        assert!((2_000..=8_000).contains(&e));
        // 0.2*8000 + 0.8*2000 = 3200
        assert_eq!(e, 3_200);
        // repeated blending toward the sample converges upward
        let mut q = 0u32;
        for _ in 0..50 {
            q = reputation_ema(q, BPS, REPUTATION_EMA_ALPHA_BPS);
        }
        assert!(q > 9_900);
    }

    #[test]
    fn reputation_decay_toward_baseline() {
        let base = REPUTATION_BASELINE_QUALITY_BPS;
        // no elapsed time → unchanged
        assert_eq!(reputation_decayed_quality_bps(9_000, 1_000, 1_000), 9_000);
        // baseline is a fixed point
        assert_eq!(reputation_decayed_quality_bps(base, 1_000_000, 0), base);
        // from above, one period pulls down toward baseline (not past it)
        let after = reputation_decayed_quality_bps(9_000, REPUTATION_DECAY_PERIOD_SECONDS, 0);
        assert!(after < 9_000 && after > base);
        // from below, pulls up toward baseline
        let up = reputation_decayed_quality_bps(0, REPUTATION_DECAY_PERIOD_SECONDS, 0);
        assert!(up > 0 && up < base);
        // very stale → fully decayed to baseline
        assert_eq!(reputation_decayed_quality_bps(9_999, i64::MAX, 0), base);
    }
}
