//! Shared relay helpers: the wall-clock source fed to the rate limiter, the metered-client
//! key derivation, and a small constructor for [`crate::node::Relay`]. The relay/exit event
//! loop itself lives in [`crate::circuit_relay`] (the persistent `/weft/circuit/1.0.0`
//! streaming data plane).

use std::time::{SystemTime, UNIX_EPOCH};

use libp2p::PeerId;

use crate::keys::WeftKeypair;
use crate::node::Relay;

/// How a relay obtains the wall-clock `now` fed to the rate limiter.
#[derive(Clone, Copy)]
pub enum Clock {
    System,
    Fixed(u64),
}

impl Clock {
    pub(crate) fn now(&self) -> u64 {
        match self {
            Clock::System => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            Clock::Fixed(t) => *t,
        }
    }
}

/// A relay's metered upstream client key = the libp2p peer id bytes (multihash → its
/// 32-byte ed25519 digest tail for our deterministic ed25519 peers).
pub(crate) fn peer_bytes(peer: &PeerId) -> [u8; 32] {
    let b = peer.to_bytes();
    let mut out = [0u8; 32];
    let tail = &b[b.len().saturating_sub(32)..];
    out[32 - tail.len()..].copy_from_slice(tail);
    out
}

/// Build a [`Relay`] from a node keypair.
pub fn make_relay(kp: &WeftKeypair, node_id: u64, window_start: u64) -> Relay {
    Relay::new(
        kp.operator_pubkey(),
        node_id,
        kp.onion_secret(),
        window_start,
    )
}
