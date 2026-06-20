//! Reward-distribution merkle tree (M4 settlement).
//!
//! Domain-separated (`0x00` leaves / `0x01` intermediates), sorted-pair, with a
//! double-hashed, **epoch-bound** leaf so a proof for one 10-minute epoch can never
//! be replayed against another epoch's root (jito-distributor + OpenZeppelin
//! hardening). SHA-256 is byte-identical to Solana `hashv` and `@noble/hashes`
//! sha256, so the on-chain program, the off-chain aggregator, and tests all agree.

use alloc::vec::Vec;

use sha2::{Digest, Sha256};

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// The merkle leaf for an epoch reward entitlement. Binds operator, node id, the
/// absolute reward amount, and the epoch; leaf domain `0x00` + double hash.
pub fn hash_reward_leaf(epoch: u64, operator: &[u8; 32], node_id: u64, amount: u64) -> [u8; 32] {
    let inner = sha256(&[
        operator,
        &node_id.to_le_bytes(),
        &amount.to_le_bytes(),
        &epoch.to_le_bytes(),
    ]);
    sha256(&[&[0u8], &inner])
}

/// The merkle leaf for an IDO/TGE allocation entitlement (M8 token distributor). Binds
/// the **distributor pubkey** (so a proof can't replay across distribution rounds), the
/// claimant, and the total allocation. Leaf domain `0x02` keeps it disjoint from reward
/// leaves (`0x00`) and intermediate nodes (`0x01`); double-hashed like the others.
pub fn hash_allocation_leaf(distributor: &[u8; 32], claimant: &[u8; 32], amount: u64) -> [u8; 32] {
    let inner = sha256(&[distributor, claimant, &amount.to_le_bytes()]);
    sha256(&[&[2u8], &inner])
}

/// Hash two nodes in sorted order with the intermediate domain prefix `0x01`.
fn hash_pair(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    if a <= b {
        sha256(&[&[1u8], &a, &b])
    } else {
        sha256(&[&[1u8], &b, &a])
    }
}

/// Verify a sorted-pair merkle proof. This is the only piece the on-chain program
/// needs (no allocation beyond the borrowed proof slice).
pub fn merkle_verify(proof: &[[u8; 32]], root: [u8; 32], leaf: [u8; 32]) -> bool {
    let mut h = leaf;
    for p in proof {
        h = hash_pair(h, *p);
    }
    h == root
}

/// Compute the next level of a tree, promoting an unpaired final node unchanged.
fn next_level(level: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let mut next = Vec::with_capacity(level.len().div_ceil(2));
    let mut i = 0;
    while i < level.len() {
        if i + 1 < level.len() {
            next.push(hash_pair(level[i], level[i + 1]));
        } else {
            next.push(level[i]);
        }
        i += 2;
    }
    next
}

/// Build the merkle root from leaves (must be non-empty).
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    while level.len() > 1 {
        level = next_level(&level);
    }
    level[0]
}

/// Build the proof (sibling path) for the leaf at `index`.
pub fn merkle_proof(leaves: &[[u8; 32]], index: usize) -> Vec<[u8; 32]> {
    let mut proof = Vec::new();
    let mut idx = index;
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    while level.len() > 1 {
        let sibling = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
        if sibling < level.len() {
            proof.push(level[sibling]);
        }
        level = next_level(&level);
        idx /= 2;
    }
    proof
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: u64) -> Vec<[u8; 32]> {
        (0..n)
            .map(|i| hash_reward_leaf(1, &[i as u8; 32], i, (i + 1) * 1000))
            .collect()
    }

    #[test]
    fn verify_round_trips_for_various_sizes() {
        for n in [1u64, 2, 3, 5, 8, 17] {
            let ls = leaves(n);
            let root = merkle_root(&ls);
            for (i, leaf) in ls.iter().enumerate() {
                let proof = merkle_proof(&ls, i);
                assert!(merkle_verify(&proof, root, *leaf), "n={n} i={i}");
            }
        }
    }

    #[test]
    fn tampered_proof_or_root_fails() {
        let ls = leaves(8);
        let root = merkle_root(&ls);
        let mut proof = merkle_proof(&ls, 3);
        assert!(merkle_verify(&proof, root, ls[3]));
        // tamper a proof element
        proof[0][0] ^= 1;
        assert!(!merkle_verify(&proof, root, ls[3]));
        // wrong leaf
        let good = merkle_proof(&ls, 3);
        assert!(!merkle_verify(&good, root, ls[4]));
        // wrong root
        let mut bad_root = root;
        bad_root[0] ^= 1;
        assert!(!merkle_verify(&good, bad_root, ls[3]));
    }

    #[test]
    fn leaf_is_epoch_bound_and_domain_separated() {
        let op = [7u8; 32];
        // same operator/node/amount, different epoch → different leaf (anti-replay)
        assert_ne!(
            hash_reward_leaf(1, &op, 1, 100),
            hash_reward_leaf(2, &op, 1, 100)
        );
        // a 64-byte intermediate value (0x01-prefixed) can't be re-presented as a leaf
        let a = hash_reward_leaf(1, &op, 1, 100);
        let b = hash_reward_leaf(1, &op, 2, 200);
        let intermediate = hash_pair(a, b);
        // there is no (epoch, operator, node, amount) producing `intermediate` because
        // leaves use the 0x00 domain; sanity-check it differs from both children.
        assert_ne!(intermediate, a);
        assert_ne!(intermediate, b);
    }

    /// Golden vector consumed by the TS parity test (chunk 5).
    #[test]
    fn golden_leaf_vector() {
        let leaf = hash_reward_leaf(42, &[2u8; 32], 7, 123_456);
        let hex: alloc::string::String = leaf.iter().map(|b| alloc::format!("{b:02x}")).collect();
        // If this changes, the TS mirror (sdk math.ts) must change in lockstep.
        assert_eq!(
            hex,
            "cea5f73a341abd012da25e67633b67f133416f1c5d045b877b8b9602bd8424f1"
        );
    }

    #[test]
    fn allocation_leaf_is_distributor_bound_and_distinct_from_reward_leaf() {
        let dist = [3u8; 32];
        let claimant = [7u8; 32];
        // bound to the distributor → a proof can't replay across rounds
        assert_ne!(
            hash_allocation_leaf(&dist, &claimant, 100),
            hash_allocation_leaf(&[9u8; 32], &claimant, 100)
        );
        // different claimant / amount → different leaf
        assert_ne!(
            hash_allocation_leaf(&dist, &claimant, 100),
            hash_allocation_leaf(&dist, &[8u8; 32], 100)
        );
        assert_ne!(
            hash_allocation_leaf(&dist, &claimant, 100),
            hash_allocation_leaf(&dist, &claimant, 101)
        );
        // domain `0x02` keeps it disjoint from a same-shaped reward leaf
        assert_ne!(
            hash_allocation_leaf(&dist, &claimant, 100),
            hash_reward_leaf(0, &dist, 0, 100)
        );
    }

    #[test]
    fn allocation_leaves_build_a_verifiable_tree() {
        let dist = [1u8; 32];
        let ls: Vec<[u8; 32]> = (0..5u64)
            .map(|i| hash_allocation_leaf(&dist, &[i as u8; 32], (i + 1) * 1000))
            .collect();
        let root = merkle_root(&ls);
        for (i, leaf) in ls.iter().enumerate() {
            assert!(merkle_verify(&merkle_proof(&ls, i), root, *leaf));
        }
    }
}
