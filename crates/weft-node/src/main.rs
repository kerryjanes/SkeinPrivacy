//! `weft-node` — the node operator daemon. Derives a stable libp2p identity from the
//! operator key, joins the Kademlia DHT over the production tcp+noise+yamux transport,
//! publishes its signed descriptor, and then runs the [`weft_net::relay::RelayService`]
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
use weft_net::relay::{self, Clock, RelayService};

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

    // Learn the actual bound address (the configured port may be 0 = auto-assigned). Fail
    // fast on a bind error (e.g. port in use) instead of hanging.
    use futures::StreamExt;
    use libp2p::swarm::SwarmEvent;
    let bound = loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => break address.to_string(),
            SwarmEvent::ListenerError { error, .. } => {
                return Err(format!("listen failed on {listen}: {error}").into());
            }
            SwarmEvent::ListenerClosed { reason, .. } => {
                return Err(format!("listener closed on {listen}: {reason:?}").into());
            }
            _ => {}
        }
    };
    let peer_id = swarm.local_peer_id().to_base58();

    // Write a bootstrap manifest so a VPN client / other relays can find us (the real,
    // distributed path). In production this comes from the on-chain registry + indexer.
    if let Ok(path) = env::var("WEFT_MANIFEST") {
        let m = weft_vpn::manifest::NodeManifest {
            operator: hex::encode(kp.operator_pubkey()),
            node_id,
            onion_pub: hex::encode(kp.onion_public()),
            static_pub: hex::encode(kp.static_public()),
            geo,
            capabilities: caps,
            peer_id: peer_id.clone(),
            multiaddr: bound.clone(),
            availability: 100,
            reputation_bps: 10_000,
        };
        m.write(std::path::Path::new(&path))?;
        println!("[weft-node] wrote manifest → {path}");
    }

    // Publish the operator-signed node descriptor keyed by sha256(operator‖node_id).
    let desc = NodeDescriptor {
        operator: kp.operator_pubkey(),
        node_id,
        multiaddr: bound.clone(),
        peer_id,
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

    // Exit handler: the real internet-egress exit. By default it dials the open internet
    // (a genuine VPN exit); set WEFT_EXIT_ALLOWLIST=ip1,ip2 to restrict egress (used for
    // scoped/test runs). Relay-only nodes never reach the exit (no EXIT capability path).
    let relay = relay::make_relay(&kp, node_id, 0);
    let policy = match env::var("WEFT_EXIT_ALLOWLIST") {
        Ok(list) if !list.trim().is_empty() => {
            let ips = list
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();
            weft_vpn::exit::EgressPolicy::allowlist(ips)
        }
        _ => weft_vpn::exit::EgressPolicy::open(),
    };
    let exit = Box::new(weft_vpn::exit::InternetExit::new(policy));
    let mut service = RelayService::new(swarm, relay, Clock::System, exit);

    // Bootstrap forwarding: seed the address book from the peer manifest directory so peeled
    // next-hops resolve to dialable peers. We wait briefly so every node has written its
    // manifest, then add a route for each peer (production relays resolve via the DHT).
    if let Ok(dir) = env::var("WEFT_PEERS") {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let own = weft_net::discovery::routing_addr(&kp.operator_pubkey(), node_id);
        if let Ok(peers) = weft_vpn::manifest::load_dir(std::path::Path::new(&dir)) {
            let mut n = 0;
            for m in &peers {
                if let (Some(ra), Some((peer, addr))) = (m.routing_addr(), m.dial()) {
                    if ra != own {
                        service.add_route(ra, peer, addr);
                        n += 1;
                    }
                }
            }
            println!("[weft-node] seeded {n} peer routes from {dir}");
        }
    }

    println!("[weft-node] relaying cells on /weft/cell/1.0.0 (real internet exit)");
    loop {
        service.step().await;
    }
}
