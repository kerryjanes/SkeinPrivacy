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

use weft_net::discovery::routing_addr;
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
