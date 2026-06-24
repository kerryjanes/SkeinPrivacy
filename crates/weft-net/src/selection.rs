//! Circuit selection (the protocol spec: "the protocol selects 3–5 nodes based on
//! geolocation, ping, and reputation"). A pure, seeded function over the node
//! directory (the M2 indexer `/nodes` shape) so circuits are reproducible in tests:
//! filter by capability + availability, weight by `reputation × geo-proximity`, and
//! draw a weighted-random path whose last hop is an EXIT.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use weft_primitives::{capability, geo_region_prefix};

use crate::error::{NetError, Result};
use crate::sphinx::MAX_HOPS;

/// A candidate node (mirrors the indexer `NodeRecord` + the DHT descriptor keys).
#[derive(Clone, Debug)]
pub struct NodeRecord {
    pub operator: [u8; 32],
    pub node_id: u64,
    pub onion_pub: [u8; 32],
    pub static_pub: [u8; 32],
    pub addr: [u8; 32],
    pub geo: u32,
    pub capabilities: u32,
    pub availability: u8,
    pub reputation_bps: u16,
}

pub struct SelectParams {
    /// Hop count (2..=MAX_HOPS).
    pub k: usize,
    pub seed: u64,
    pub min_availability: u8,
    pub client_geo: u32,
}

/// Shared top-region length (0..=6); higher = geographically closer.
fn geo_proximity(a: u32, b: u32) -> u32 {
    for chars in (1..=6u8).rev() {
        if geo_region_prefix(a, chars) == geo_region_prefix(b, chars) {
            return chars as u32;
        }
    }
    0
}

fn weight(n: &NodeRecord, client_geo: u32) -> u64 {
    let rep = n.reputation_bps.max(1) as u64;
    let geo = (geo_proximity(client_geo, n.geo) + 1) as u64; // 1..=7
    rep * geo
}

fn weighted_pick(rng: &mut StdRng, pool: &[(usize, u64)]) -> Option<usize> {
    let total: u64 = pool.iter().map(|(_, w)| *w).sum();
    if total == 0 {
        return None;
    }
    let mut r = rng.gen_range(0..total);
    for (idx, w) in pool {
        if r < *w {
            return Some(*idx);
        }
        r -= *w;
    }
    pool.last().map(|(i, _)| *i)
}

/// Select an ordered circuit: `k-1` RELAY middle hops + 1 EXIT last hop. Deterministic
/// in `seed`. Errors if no candidate set satisfies the constraints.
pub fn select_circuit(nodes: &[NodeRecord], p: &SelectParams) -> Result<Vec<NodeRecord>> {
    if !(2..=MAX_HOPS).contains(&p.k) {
        return Err(NetError::Circuit("hop count must be 2..=MAX_HOPS"));
    }
    let mut rng = StdRng::seed_from_u64(p.seed);

    let eligible: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            n.availability >= p.min_availability && n.capabilities & capability::WIREGUARD != 0
        })
        .map(|(i, _)| i)
        .collect();

    let exit_pool: Vec<(usize, u64)> = eligible
        .iter()
        .filter(|&&i| nodes[i].capabilities & capability::EXIT != 0)
        .map(|&i| (i, weight(&nodes[i], p.client_geo)))
        .collect();
    let exit = weighted_pick(&mut rng, &exit_pool).ok_or(NetError::NoCandidates)?;

    let mut used = vec![exit];
    let mut middles = Vec::new();
    while middles.len() < p.k - 1 {
        let relay_pool: Vec<(usize, u64)> = eligible
            .iter()
            .filter(|&&i| !used.contains(&i) && nodes[i].capabilities & capability::RELAY != 0)
            .map(|&i| (i, weight(&nodes[i], p.client_geo)))
            .collect();
        let pick = weighted_pick(&mut rng, &relay_pool).ok_or(NetError::NoCandidates)?;
        used.push(pick);
        middles.push(pick);
    }

    let mut path: Vec<NodeRecord> = middles.iter().map(|&i| nodes[i].clone()).collect();
    path.push(nodes[exit].clone());
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tag: u8, caps: u32, rep: u16, geo: u32, avail: u8) -> NodeRecord {
        NodeRecord {
            operator: [tag; 32],
            node_id: tag as u64,
            onion_pub: [tag; 32],
            static_pub: [tag; 32],
            addr: [tag; 32],
            geo,
            capabilities: caps,
            availability: avail,
            reputation_bps: rep,
        }
    }

    fn directory() -> Vec<NodeRecord> {
        let relay = capability::WIREGUARD | capability::RELAY;
        let exit = capability::WIREGUARD | capability::RELAY | capability::EXIT;
        vec![
            rec(1, relay, 8_000, 100, 90),
            rec(2, relay, 12_000, 100, 90),
            rec(3, relay, 6_000, 200, 80),
            rec(4, exit, 15_000, 100, 95),
            rec(5, exit, 9_000, 300, 70),
            rec(6, relay, 10_000, 100, 50),
        ]
    }

    #[test]
    fn selects_k_hops_with_exit_last() {
        let dir = directory();
        let p = SelectParams {
            k: 3,
            seed: 42,
            min_availability: 60,
            client_geo: 100,
        };
        let path = select_circuit(&dir, &p).unwrap();
        assert_eq!(path.len(), 3);
        assert!(path.last().unwrap().capabilities & capability::EXIT != 0);
        // all distinct
        let ids: std::collections::HashSet<_> = path.iter().map(|n| n.node_id).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn deterministic_in_seed() {
        let dir = directory();
        let p = SelectParams {
            k: 3,
            seed: 7,
            min_availability: 0,
            client_geo: 100,
        };
        let a = select_circuit(&dir, &p).unwrap();
        let b = select_circuit(&dir, &p).unwrap();
        assert_eq!(
            a.iter().map(|n| n.node_id).collect::<Vec<_>>(),
            b.iter().map(|n| n.node_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn availability_filter_can_starve() {
        let dir = directory();
        // require 99% availability → no node qualifies
        let p = SelectParams {
            k: 3,
            seed: 1,
            min_availability: 99,
            client_geo: 100,
        };
        assert!(select_circuit(&dir, &p).is_err());
    }

    #[test]
    fn higher_reputation_chosen_more_often() {
        let dir = directory();
        // exit 4 (rep 15000, geo 100) vs exit 5 (rep 9000, geo 300); client_geo 100
        let mut count4 = 0;
        for seed in 0..200 {
            let p = SelectParams {
                k: 3,
                seed,
                min_availability: 60,
                client_geo: 100,
            };
            let path = select_circuit(&dir, &p).unwrap();
            if path.last().unwrap().node_id == 4 {
                count4 += 1;
            }
        }
        assert!(
            count4 > 130,
            "high-rep+near exit should dominate: {count4}/200"
        );
    }
}
