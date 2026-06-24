//! Relay/exit node logic: peel one onion layer, enforce per-client rate limiting and
//! (at exits) content opt-out, meter the forwarded bytes, and at the window boundary
//! emit signed receipt cores. Pure over the cell stream so the simulator and the
//! `weft-node` daemon share the same code.

use std::collections::HashMap;

use curve25519_dalek::scalar::Scalar;

use crate::error::{NetError, Result};
use crate::metering::{CircuitMeter, TokenBucket};
use crate::receipt::ReceiptCore;
use crate::sphinx::{peel, Cell, Peeled};

/// Exit-node content policy: destinations the operator has opted out of serving
/// (the protocol spec "opt-out mechanism for node operators by content type").
#[derive(Default)]
pub struct ExitPolicy {
    blocked: Vec<[u8; 32]>,
}

impl ExitPolicy {
    pub fn block(&mut self, dest: [u8; 32]) {
        self.blocked.push(dest);
    }
    pub fn allows(&self, dest: &[u8; 32]) -> bool {
        !self.blocked.contains(dest)
    }
}

pub struct Relay {
    operator: [u8; 32],
    node_id: u64,
    onion_secret: Scalar,
    window_start: u64,
    bucket_capacity: u64,
    bucket_refill: u64,
    next_nonce: u64,
    meters: HashMap<[u8; 32], CircuitMeter>,
    buckets: HashMap<[u8; 32], TokenBucket>,
    policy: ExitPolicy,
}

impl Relay {
    pub fn new(operator: [u8; 32], node_id: u64, onion_secret: Scalar, window_start: u64) -> Self {
        Self {
            operator,
            node_id,
            onion_secret,
            window_start,
            bucket_capacity: 64 * 1024 * 1024, // 64 MB burst
            bucket_refill: 8 * 1024 * 1024,    // 8 MB/s sustained
            next_nonce: 1,
            meters: HashMap::new(),
            buckets: HashMap::new(),
            policy: ExitPolicy::default(),
        }
    }

    pub fn with_rate_limit(mut self, capacity: u64, refill_per_sec: u64) -> Self {
        self.bucket_capacity = capacity;
        self.bucket_refill = refill_per_sec;
        self
    }

    pub fn exit_policy_mut(&mut self) -> &mut ExitPolicy {
        &mut self.policy
    }

    /// Process one cell for `client` at time `now`. Rate-limits, peels, enforces exit
    /// policy, and meters the cell's wire bytes.
    pub fn process(&mut self, client: [u8; 32], cell: &Cell, now: u64) -> Result<Peeled> {
        let wire = cell.wire_len() as u64;
        let bucket = self
            .buckets
            .entry(client)
            .or_insert_with(|| TokenBucket::new(self.bucket_capacity, self.bucket_refill, now));
        if !bucket.admit(wire, now) {
            return Err(NetError::RateLimited);
        }

        let peeled = peel(cell, &self.onion_secret)?;
        if let Peeled::Exit { dest, .. } = &peeled {
            if !self.policy.allows(dest) {
                return Err(NetError::ContentOptOut);
            }
        }

        self.meters
            .entry(client)
            .or_insert_with(|| {
                CircuitMeter::new(client, self.operator, self.node_id, self.window_start)
            })
            .record(wire);
        Ok(peeled)
    }

    pub fn metered_bytes(&self, client: &[u8; 32]) -> u64 {
        self.meters.get(client).map(|m| m.bytes()).unwrap_or(0)
    }

    /// Total bytes metered across all upstream clients this window.
    pub fn metered_total(&self) -> u64 {
        self.meters.values().map(|m| m.bytes()).sum()
    }

    /// Number of distinct upstream clients metered this window.
    pub fn client_count(&self) -> usize {
        self.meters.len()
    }

    /// Close every open circuit meter into receipt cores (one per client) at window end.
    pub fn close_window(&mut self, window_end: u64) -> Vec<ReceiptCore> {
        let mut out = Vec::new();
        for (_, meter) in self.meters.drain() {
            let nonce = self.next_nonce;
            self.next_nonce += 1;
            out.push(meter.into_core(window_end, nonce));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::WeftKeypair;
    use crate::sphinx::{create_onion, OnionHop};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn relay_for(kp: &WeftKeypair) -> Relay {
        Relay::new(kp.operator_pubkey(), 1, kp.onion_secret(), 600)
    }

    #[test]
    fn forwards_meters_and_emits_receipt() {
        let mut rng = StdRng::seed_from_u64(1);
        let hop_kp = WeftKeypair::generate(&mut rng);
        let exit_kp = WeftKeypair::generate(&mut rng);
        let client = [9u8; 32];
        let dest = [0xde; 32];

        let hops = vec![
            OnionHop {
                onion_pub: hop_kp.onion_public(),
                addr: exit_kp.onion_public(),
                is_exit: false,
            },
            OnionHop {
                onion_pub: exit_kp.onion_public(),
                addr: dest,
                is_exit: true,
            },
        ];
        let (cell, _keys) = create_onion(&mut rng, &hops, b"payload").unwrap();

        let mut relay = relay_for(&hop_kp);
        let wire = cell.wire_len() as u64;
        let next = match relay.process(client, &cell, 0).unwrap() {
            Peeled::Forward { cell, .. } => cell,
            _ => panic!("first hop should forward"),
        };
        assert_eq!(relay.metered_bytes(&client), wire);

        let mut exit = relay_for(&exit_kp);
        match exit.process(client, &next, 0).unwrap() {
            Peeled::Exit {
                dest: d, payload, ..
            } => {
                assert_eq!(d, dest);
                assert_eq!(payload, b"payload");
            }
            _ => panic!("exit should deliver"),
        }
        let cores = relay.close_window(1_200);
        assert_eq!(cores.len(), 1);
        assert_eq!(cores[0].bytes, wire);
        assert_eq!(cores[0].operator, hop_kp.operator_pubkey());
    }

    #[test]
    fn rate_limit_drops_excess() {
        let mut rng = StdRng::seed_from_u64(2);
        let hop_kp = WeftKeypair::generate(&mut rng);
        let exit_kp = WeftKeypair::generate(&mut rng);
        let hops = vec![
            OnionHop {
                onion_pub: hop_kp.onion_public(),
                addr: exit_kp.onion_public(),
                is_exit: false,
            },
            OnionHop {
                onion_pub: exit_kp.onion_public(),
                addr: [0xde; 32],
                is_exit: true,
            },
        ];
        let (cell, _) = create_onion(&mut rng, &hops, b"x").unwrap();
        // capacity below one cell → first admit fails
        let mut relay = relay_for(&hop_kp).with_rate_limit(10, 1);
        assert!(matches!(
            relay.process([9; 32], &cell, 0),
            Err(NetError::RateLimited)
        ));
    }

    #[test]
    fn exit_refuses_opted_out_destination() {
        let mut rng = StdRng::seed_from_u64(3);
        let exit_kp = WeftKeypair::generate(&mut rng);
        let dest = [0xde; 32];
        let hops = vec![OnionHop {
            onion_pub: exit_kp.onion_public(),
            addr: dest,
            is_exit: true,
        }];
        let (cell, _) = create_onion(&mut rng, &hops, b"x").unwrap();
        let mut exit = relay_for(&exit_kp);
        exit.exit_policy_mut().block(dest);
        assert!(matches!(
            exit.process([9; 32], &cell, 0),
            Err(NetError::ContentOptOut)
        ));
    }
}
