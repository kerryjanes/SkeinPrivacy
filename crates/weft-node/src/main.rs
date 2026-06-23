//! `weft-node` — the node operator daemon. Derives a stable libp2p identity from the
//! operator key, joins the Kademlia DHT over the production tcp+noise+yamux transport,
//! publishes its signed descriptor, and then runs the [`relay_service::RelayService`]
//! event loop so it relays onion cells over the `/weft/cell/1.0.0` transport. Real exit
//! egress to the internet (OS TUN / socket routing) is the deferred M8 step; this build
//! delivers an in-protocol ack at the exit.

use std::env;

use libp2p::kad;
use rand::rngs::StdRng;
use rand::SeedableRng;
use weft_net::discovery::{
    build_swarm, descriptor_signing_bytes, record_key, sign_descriptor, NodeDescriptor,
};
use weft_net::keys::WeftKeypair;

mod daemon;
mod relay_service;
use relay_service::{Clock, RelayService};

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

    // Build the swarm with the operator-derived libp2p identity, listen, and join the DHT.
    let mut swarm = build_swarm(kp.libp2p_keypair(), false)?;
    swarm.listen_on(listen.parse()?)?;
    swarm.behaviour_mut().kad.set_mode(Some(kad::Mode::Server));

    // Publish the operator-signed node descriptor keyed by sha256(operator‖node_id).
    let desc = NodeDescriptor {
        operator: kp.operator_pubkey(),
        node_id,
        multiaddr: listen.clone(),
        peer_id: swarm.local_peer_id().to_base58(),
        noise_static_pub: kp.static_public(),
        onion_pub: kp.onion_public(),
        capabilities: caps,
        geo,
        issued_at: 0,
    };
    let sig = kp.sign(&descriptor_signing_bytes(&desc));
    swarm.behaviour_mut().kad.put_record(
        kad::Record {
            key: record_key(&kp.operator_pubkey(), node_id),
            value: sign_descriptor(desc, sig),
            publisher: None,
            expires: None,
        },
        kad::Quorum::One,
    )?;
    println!("[weft-node] joined the DHT; serving node {node_id} on {listen}");

    // Exit handler. The real internet-egress exit (weft-vpn `InternetExit`) is wired in the
    // next M9 chunk; for now use the echo exit so the reverse path is exercised end to end.
    let relay = relay_service::make_relay(&kp, node_id, 0);
    let exit = Box::new(weft_net::exit::EchoExit);
    let mut service = RelayService::new(swarm, relay, Clock::System, exit);
    println!("[weft-node] relaying cells on /weft/cell/1.0.0");
    loop {
        service.step().await;
    }
}
