//! Property-based (proptest) fuzzing of the economic integer math (M8 audit hardening).
//! These assert the invariants an auditor cares about — no panics, conservation,
//! monotonicity, and bounds — across the whole input space, complementing the
//! example-based unit tests in the crate.

use proptest::prelude::*;
use weft_primitives::{
    merkle::{hash_allocation_leaf, merkle_proof, merkle_root, merkle_verify},
    split_payment_bps, split_tge, traffic_reward, traffic_reward_with_bootstrap, vested_amount,
    BPS,
};

proptest! {
    /// `traffic_reward` never panics and is monotonic non-decreasing in `bytes`.
    #[test]
    fn traffic_reward_is_monotonic_in_bytes(
        a in 0u64..=2_000_000_000_000,
        b in 0u64..=2_000_000_000_000,
        rep in 0u32..=30_000,
        geo in 0u32..=10_000,
        stk in 0u32..=5_000,
    ) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let rlo = traffic_reward(lo, rep, geo, stk);
        let rhi = traffic_reward(hi, rep, geo, stk);
        prop_assert!(rhi >= rlo);
    }

    /// The bootstrap variant is always ≥ the base reward (the bonus only adds).
    #[test]
    fn bootstrap_never_reduces_reward(
        bytes in 0u64..=1_000_000_000_000,
        rep in 5_000u32..=20_000,
        geo in 0u32..=5_000,
        stk in 0u32..=2_000,
        boot in 0u32..=15_000,
    ) {
        let base = traffic_reward(bytes, rep, geo, stk);
        let boosted = traffic_reward_with_bootstrap(bytes, rep, geo, stk, boot);
        prop_assert!(boosted >= base);
    }

    /// A payment split always conserves the total exactly (no leaked/minted units).
    #[test]
    fn split_payment_conserves_total(
        amount in 0u64..=u64::MAX,
        nodes_bps in 0u32..=10_000,
        burn_bps in 0u32..=10_000,
    ) {
        // keep nodes+burn within range so treasury (= remainder) is non-negative
        let nodes_bps = nodes_bps.min(BPS);
        let burn_bps = burn_bps.min(BPS - nodes_bps);
        let s = split_payment_bps(amount, nodes_bps, burn_bps);
        prop_assert_eq!(s.nodes as u128 + s.burn as u128 + s.treasury as u128, amount as u128);
    }

    /// A TGE split always conserves the allocation exactly.
    #[test]
    fn split_tge_conserves(allocation in 0u64..=u64::MAX, tge_bps in 0u32..=20_000) {
        let (tge, vest) = split_tge(allocation, tge_bps);
        prop_assert_eq!(tge as u128 + vest as u128, allocation as u128);
    }

    /// `vested_amount` is monotonic non-decreasing in time, never exceeds `total`, and
    /// reaches exactly `total` once fully elapsed.
    #[test]
    fn vested_amount_is_bounded_and_monotonic(
        total in 0u64..=1_000_000_000_000_000,
        cliff_unlock in 0u64..=1_000_000_000_000_000,
        start in -1_000_000_000i64..=1_000_000_000,
        cliff_raw in 0i64..=126_144_000,
        dur in 1i64..=126_144_000,        // up to 4y
        t0 in -2_000_000_000i64..=2_000_000_000,
        dt in 0i64..=126_144_000,
    ) {
        // A well-formed schedule has its cliff within its total duration.
        let cliff = cliff_raw.min(dur);
        let t1 = t0.saturating_add(dt);
        let v0 = vested_amount(total, cliff_unlock, start, cliff, dur, t0);
        let v1 = vested_amount(total, cliff_unlock, start, cliff, dur, t1);
        prop_assert!(v1 >= v0, "monotonic: {v0} -> {v1}");
        prop_assert!(v1 <= total, "never over total");
        // fully past the schedule end → exactly total
        let end = start.saturating_add(dur);
        let vend = vested_amount(total, cliff_unlock, start, cliff, dur, end.saturating_add(1));
        prop_assert_eq!(vend, total);
    }

    /// A random allocation tree's every leaf verifies against its root.
    #[test]
    fn allocation_merkle_round_trips(
        dist in any::<[u8; 32]>(),
        amounts in prop::collection::vec(0u64..=u64::MAX, 1..16),
    ) {
        let leaves: Vec<[u8; 32]> = amounts
            .iter()
            .enumerate()
            .map(|(i, a)| hash_allocation_leaf(&dist, &[i as u8; 32], *a))
            .collect();
        let root = merkle_root(&leaves);
        for (i, leaf) in leaves.iter().enumerate() {
            prop_assert!(merkle_verify(&merkle_proof(&leaves, i), root, *leaf));
        }
    }
}
