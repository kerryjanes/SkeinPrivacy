//! Node discovery over a libp2p **Kademlia** DHT (`SPEC.md`: "Kademlia DHT for node
//! discovery"). Each node PUTs a self-authenticating, operator-signed [`NodeDescriptor`]
//! keyed by `sha256(operator ‖ node_id_le)` — the same preimage as the on-chain
//! `NodeState.endpoint_hash`, so a client that read the chain can derive the DHT key and
//! cross-check the descriptor against the on-chain commitment. The on-chain
//! `endpoint_hash` commits to the *stable* identity fields (keys/caps/geo), not the
//! volatile multiaddr; a descriptor inconsistent with the commitment is operator-signed,
//! attributable evidence for a dispute.

use libp2p::identity;
use libp2p::kad::{self, store::MemoryStore, RecordKey};
use libp2p::swarm::NetworkBehaviour;
use libp2p::{identify, Swarm, SwarmBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::NetError;
use crate::keys::verify_sig;

// NB: we intentionally do NOT bring a one-parameter `Result` into module scope — the
// `NetworkBehaviour` derive macro generates code using std `Result<_, ConnectionDenied>`,
// so our fallible fns spell out `crate::error::Result<T>` explicitly.

/// The DHT-published node descriptor (resolves identity → live endpoint + keys).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDescriptor {
    pub operator: [u8; 32],
    pub node_id: u64,
    pub multiaddr: String,
    /// The node's libp2p `PeerId` (base58). Deterministically derived from `operator`,
    /// so `validate_record` rejects any descriptor advertising a mismatched peer id.
    pub peer_id: String,
    pub noise_static_pub: [u8; 32],
    pub onion_pub: [u8; 32],
    pub capabilities: u32,
    pub geo: u32,
    pub issued_at: i64,
}

/// A descriptor plus the operator's signature over its canonical bytes. The sig is a
/// `Vec<u8>` (serde derives arrays only up to length 32).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedDescriptor {
    pub desc: NodeDescriptor,
    pub sig: Vec<u8>,
}

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// The DHT record key for a node: `sha256(operator ‖ node_id_le)`.
pub fn record_key(operator: &[u8; 32], node_id: u64) -> RecordKey {
    RecordKey::new(&sha256(&[operator, &node_id.to_le_bytes()]))
}

/// The on-chain `endpoint_hash` commitment: `sha256` over the STABLE identity fields
/// (not the volatile multiaddr). Written to `NodeState.endpoint_hash` at registration;
/// re-derivable by a client from a DHT descriptor to detect inconsistency.
pub fn endpoint_commitment(desc: &NodeDescriptor) -> [u8; 32] {
    sha256(&[
        &desc.operator,
        &desc.node_id.to_le_bytes(),
        &desc.noise_static_pub,
        &desc.onion_pub,
        &desc.capabilities.to_le_bytes(),
        &desc.geo.to_le_bytes(),
    ])
}

/// Canonical bytes the operator signs (deterministic bincode of the descriptor).
fn signing_bytes(desc: &NodeDescriptor) -> Vec<u8> {
    bincode::serialize(desc).expect("descriptor serialize")
}

/// Build a signed DHT record value for this descriptor.
pub fn sign_descriptor(desc: NodeDescriptor, sig: [u8; 64]) -> Vec<u8> {
    bincode::serialize(&SignedDescriptor {
        desc,
        sig: sig.to_vec(),
    })
    .expect("signed descriptor serialize")
}

/// Bytes the operator must sign to publish `desc`.
pub fn descriptor_signing_bytes(desc: &NodeDescriptor) -> Vec<u8> {
    signing_bytes(desc)
}

/// Validate an inbound DHT record: the key must match `sha256(operator ‖ node_id)` and
/// the operator signature must verify. Returns the authenticated descriptor.
pub fn validate_record(key: &RecordKey, value: &[u8]) -> crate::error::Result<NodeDescriptor> {
    let signed: SignedDescriptor =
        bincode::deserialize(value).map_err(|_| NetError::Malformed("dht record decode"))?;
    let expect = record_key(&signed.desc.operator, signed.desc.node_id);
    if key.as_ref() != expect.as_ref() {
        return Err(NetError::Malformed("dht key mismatch"));
    }
    let sig: [u8; 64] = signed
        .sig
        .as_slice()
        .try_into()
        .map_err(|_| NetError::Malformed("sig length"))?;
    if !verify_sig(&signed.desc.operator, &signing_bytes(&signed.desc), &sig) {
        return Err(NetError::BadReceiptSig);
    }
    // The advertised peer id must be the one deterministically derived from the operator
    // key, so a node cannot point clients at a libp2p identity it doesn't control.
    let expect_peer = crate::keys::peer_id_for_operator(&signed.desc.operator)
        .ok_or(NetError::Malformed("operator key"))?;
    if signed.desc.peer_id != expect_peer.to_base58() {
        return Err(NetError::Malformed("peer id mismatch"));
    }
    Ok(signed.desc)
}

/// The libp2p behaviour: Kademlia (record store) + identify (so Kademlia learns
/// reachable listen addresses) + the `/weft/cell/1.0.0` cell transport (M7).
#[derive(NetworkBehaviour)]
pub struct WeftBehaviour {
    pub kad: kad::Behaviour<MemoryStore>,
    pub identify: identify::Behaviour,
    pub cell: crate::cell_transport::CellBehaviour,
}

fn make_behaviour(key: &identity::Keypair) -> WeftBehaviour {
    let peer_id = key.public().to_peer_id();
    WeftBehaviour {
        kad: kad::Behaviour::new(peer_id, MemoryStore::new(peer_id)),
        identify: identify::Behaviour::new(identify::Config::new(
            "/weft/1.0.0".into(),
            key.public(),
        )),
        cell: crate::cell_transport::cell_behaviour(),
    }
}

/// Build a swarm. `memory = true` uses libp2p's in-process `MemoryTransport`
/// (deterministic, socket-free — used by the simulator and CI); `false` uses the
/// production `tcp + noise + yamux` stack.
pub fn build_swarm(
    id_keys: identity::Keypair,
    memory: bool,
) -> crate::error::Result<Swarm<WeftBehaviour>> {
    // Keep idle connections alive long enough for Kademlia queries to complete (the
    // default closes idle links almost immediately, racing in-process queries).
    let idle = |cfg: libp2p::swarm::Config| {
        cfg.with_idle_connection_timeout(std::time::Duration::from_secs(60))
    };
    let swarm = if memory {
        SwarmBuilder::with_existing_identity(id_keys)
            .with_tokio()
            .with_other_transport(|key| {
                use libp2p::core::transport::MemoryTransport;
                use libp2p::core::upgrade;
                use libp2p::{noise, yamux, Transport};
                Ok(MemoryTransport::default()
                    .upgrade(upgrade::Version::V1)
                    .authenticate(
                        noise::Config::new(key).map_err(|e| NetError::Noise(format!("{e:?}")))?,
                    )
                    .multiplex(yamux::Config::default()))
            })
            .map_err(|e| NetError::Noise(format!("{e:?}")))?
            .with_behaviour(make_behaviour)
            .map_err(|e| NetError::Noise(format!("{e:?}")))?
            .with_swarm_config(idle)
            .build()
    } else {
        SwarmBuilder::with_existing_identity(id_keys)
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )
            .map_err(|e| NetError::Noise(format!("{e:?}")))?
            .with_behaviour(make_behaviour)
            .map_err(|e| NetError::Noise(format!("{e:?}")))?
            .with_swarm_config(idle)
            .build()
    };
    Ok(swarm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::WeftKeypair;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn descriptor(kp: &WeftKeypair) -> NodeDescriptor {
        NodeDescriptor {
            operator: kp.operator_pubkey(),
            node_id: 1,
            multiaddr: "/memory/42".into(),
            peer_id: kp.libp2p_keypair().public().to_peer_id().to_base58(),
            noise_static_pub: kp.static_public(),
            onion_pub: kp.onion_public(),
            capabilities: 0b111,
            geo: 12345,
            issued_at: 1700,
        }
    }

    #[test]
    fn peer_id_is_deterministic_and_distinct() {
        let mut rng = StdRng::seed_from_u64(11);
        let a = WeftKeypair::generate(&mut rng);
        let b = WeftKeypair::generate(&mut rng);
        // stable across derivations from the same operator key
        assert_eq!(
            a.libp2p_keypair().public().to_peer_id(),
            a.libp2p_keypair().public().to_peer_id()
        );
        // derivable from just the operator pubkey, and matches the keypair's peer id
        assert_eq!(
            crate::keys::peer_id_for_operator(&a.operator_pubkey()).unwrap(),
            a.libp2p_keypair().public().to_peer_id()
        );
        // distinct operators → distinct peer ids
        assert_ne!(
            a.libp2p_keypair().public().to_peer_id(),
            b.libp2p_keypair().public().to_peer_id()
        );
    }

    #[test]
    fn mismatched_peer_id_is_rejected() {
        let mut rng = StdRng::seed_from_u64(12);
        let kp = WeftKeypair::generate(&mut rng);
        let mut desc = descriptor(&kp);
        desc.peer_id = "12D3KooWuNonsensePeerIdThatDoesntMatch00000000000".into();
        let sig = kp.sign(&descriptor_signing_bytes(&desc));
        let value = sign_descriptor(desc, sig);
        let key = record_key(&kp.operator_pubkey(), 1);
        assert!(validate_record(&key, &value).is_err());
    }

    #[test]
    fn signed_record_round_trips_and_validates() {
        let mut rng = StdRng::seed_from_u64(1);
        let kp = WeftKeypair::generate(&mut rng);
        let desc = descriptor(&kp);
        let sig = kp.sign(&descriptor_signing_bytes(&desc));
        let value = sign_descriptor(desc.clone(), sig);
        let key = record_key(&kp.operator_pubkey(), 1);
        let got = validate_record(&key, &value).unwrap();
        assert_eq!(got, desc);
    }

    #[test]
    fn tampered_record_is_rejected() {
        let mut rng = StdRng::seed_from_u64(2);
        let kp = WeftKeypair::generate(&mut rng);
        let mut desc = descriptor(&kp);
        let sig = kp.sign(&descriptor_signing_bytes(&desc));
        // change a stable field after signing → signature no longer matches
        desc.geo = 999;
        let value = sign_descriptor(desc.clone(), sig);
        let key = record_key(&kp.operator_pubkey(), 1);
        assert!(validate_record(&key, &value).is_err());
    }

    #[test]
    fn wrong_key_is_rejected() {
        let mut rng = StdRng::seed_from_u64(3);
        let kp = WeftKeypair::generate(&mut rng);
        let desc = descriptor(&kp);
        let sig = kp.sign(&descriptor_signing_bytes(&desc));
        let value = sign_descriptor(desc, sig);
        let wrong = record_key(&kp.operator_pubkey(), 2); // node_id 2 ≠ 1
        assert!(validate_record(&wrong, &value).is_err());
    }

    #[test]
    fn endpoint_commitment_is_stable_field_only() {
        let mut rng = StdRng::seed_from_u64(4);
        let kp = WeftKeypair::generate(&mut rng);
        let mut desc = descriptor(&kp);
        let c1 = endpoint_commitment(&desc);
        // changing only the multiaddr must NOT change the commitment
        desc.multiaddr = "/ip4/1.2.3.4/tcp/9000".into();
        assert_eq!(c1, endpoint_commitment(&desc));
        // changing a stable field (onion key) MUST change it
        desc.onion_pub = [9u8; 32];
        assert_ne!(c1, endpoint_commitment(&desc));
    }
}
