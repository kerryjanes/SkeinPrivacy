//! Node bootstrap manifests. A `weft-node` writes one of these on startup; the VPN client
//! (and other relays) load a directory of them to learn the live network — each node's
//! routing address, onion/link keys, libp2p peer id, and dialable endpoint. This is the
//! real, distributed path: the client connects to *external* relay/exit processes over TCP,
//! not an in-process circuit. (In production the directory comes from the on-chain registry
//! + indexer; the manifest dir is the bootstrap/known-nodes equivalent.)

use std::io;
use std::path::Path;
use std::str::FromStr;

use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};

use weft_net::discovery::{endpoint_commitment_parts, routing_addr};
use weft_net::selection::NodeRecord;

/// A single node's bootstrap descriptor (JSON on disk).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeManifest {
    pub operator: String, // hex(32)
    pub node_id: u64,
    pub onion_pub: String,  // hex(32)
    pub static_pub: String, // hex(32)
    pub geo: u32,
    pub capabilities: u32,
    pub peer_id: String,   // base58 libp2p peer id
    pub multiaddr: String, // dialable endpoint, e.g. /ip4/1.2.3.4/tcp/9001
    /// Availability % and reputation (bps) come from on-chain in production; defaulted here.
    #[serde(default = "default_availability")]
    pub availability: u8,
    #[serde(default = "default_reputation")]
    pub reputation_bps: u16,
    /// The on-chain `endpoint_hash` commitment (hex) over this node's stable identity fields.
    /// The operator registers it in `NodeState.endpoint_hash`; a client can re-derive it from
    /// the DHT descriptor to detect tampering. Empty in legacy manifests.
    #[serde(default)]
    pub endpoint_hash: String,
}

fn default_availability() -> u8 {
    100
}
fn default_reputation() -> u16 {
    10_000
}

fn hex32(s: &str) -> Option<[u8; 32]> {
    let v = hex::decode(s).ok()?;
    v.try_into().ok()
}

impl NodeManifest {
    /// The on-chain routing address (DHT key) for this node.
    pub fn routing_addr(&self) -> Option<[u8; 32]> {
        Some(routing_addr(&hex32(&self.operator)?, self.node_id))
    }

    /// Derive the on-chain `endpoint_hash` commitment from this manifest's stable fields
    /// (the same value the daemon writes to `endpoint_hash`). Returns `None` on bad hex.
    pub fn endpoint_commitment(&self) -> Option<[u8; 32]> {
        Some(endpoint_commitment_parts(
            &hex32(&self.operator)?,
            self.node_id,
            &hex32(&self.static_pub)?,
            &hex32(&self.onion_pub)?,
            self.capabilities,
            self.geo,
        ))
    }

    /// Build the selection record used when constructing circuits.
    pub fn record(&self) -> Option<NodeRecord> {
        Some(NodeRecord {
            operator: hex32(&self.operator)?,
            node_id: self.node_id,
            onion_pub: hex32(&self.onion_pub)?,
            static_pub: hex32(&self.static_pub)?,
            addr: self.routing_addr()?,
            geo: self.geo,
            capabilities: self.capabilities,
            availability: self.availability,
            reputation_bps: self.reputation_bps,
        })
    }

    /// The libp2p peer + endpoint to dial this node.
    pub fn dial(&self) -> Option<(PeerId, Multiaddr)> {
        Some((
            PeerId::from_str(&self.peer_id).ok()?,
            Multiaddr::from_str(&self.multiaddr).ok()?,
        ))
    }

    pub fn write(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_commitment_matches_daemon_value() {
        // A manifest's recomputed commitment must equal the hex the daemon would write via
        // `discovery::endpoint_commitment_parts`, so the operator registers a consistent hash.
        let operator = [7u8; 32];
        let static_pub = [9u8; 32];
        let onion_pub = [11u8; 32];
        let m = NodeManifest {
            operator: hex::encode(operator),
            node_id: 42,
            onion_pub: hex::encode(onion_pub),
            static_pub: hex::encode(static_pub),
            geo: 100,
            capabilities: 7,
            peer_id: "x".into(),
            multiaddr: "/ip4/1.2.3.4/tcp/9".into(),
            availability: 100,
            reputation_bps: 10_000,
            endpoint_hash: String::new(),
        };
        let expect = endpoint_commitment_parts(&operator, 42, &static_pub, &onion_pub, 7, 100);
        assert_eq!(m.endpoint_commitment().unwrap(), expect);
        // Round-trips through JSON with the field present.
        let json = serde_json::to_string(&NodeManifest {
            endpoint_hash: hex::encode(expect),
            ..m.clone()
        })
        .unwrap();
        let back: NodeManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.endpoint_hash, hex::encode(expect));
        // Legacy manifests with no endpoint_hash still load (serde default).
        assert!(serde_json::from_str::<NodeManifest>(&json.replace(
            &format!(",\n  \"endpoint_hash\": \"{}\"", hex::encode(expect)),
            ""
        ))
        .is_ok());
    }
}

/// Load every `*.json` manifest in `dir`.
pub fn load_dir(dir: &Path) -> io::Result<Vec<NodeManifest>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let bytes = std::fs::read(&path)?;
            if let Ok(m) = serde_json::from_slice::<NodeManifest>(&bytes) {
                out.push(m);
            }
        }
    }
    Ok(out)
}
