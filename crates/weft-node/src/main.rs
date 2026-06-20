//! `weft-node` — the node operator daemon. Generates (or would load) the node
//! identity, joins the Kademlia DHT over the production tcp+noise+yamux transport,
//! and publishes its signed descriptor so clients can resolve its live endpoint and
//! keys. The onion relay/exit engine (`weft_net::node::Relay`) is ready to bind to
//! the cell transport in M7.

use std::env;

use rand::rngs::StdRng;
use rand::SeedableRng;
use weft_net::keys::WeftKeypair;

mod daemon;
mod relay_service;
use daemon::Daemon;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node_id: u64 = env::var("WEFT_NODE_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let geo: u32 = env::var("WEFT_NODE_GEO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let listen = env::var("WEFT_NODE_LISTEN").unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/0".into());
    let caps = weft_primitives::capability::WIREGUARD
        | weft_primitives::capability::RELAY
        | weft_primitives::capability::EXIT;

    // NB: a production deployment loads a persisted operator key; we generate one here.
    let mut rng = StdRng::from_entropy();
    let kp = WeftKeypair::generate(&mut rng);
    println!(
        "[weft-node] operator {}",
        hex::encode(kp.operator_pubkey())
    );

    let mut daemon = Daemon::new(kp, node_id, geo, caps, listen.parse()?, false)?;
    daemon.listen()?;
    daemon.publish(0)?;
    println!("[weft-node] joined the DHT; serving node {node_id} on {listen}");

    loop {
        let _ = daemon.next_event().await;
    }
}
