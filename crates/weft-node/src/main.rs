//! `weft-node` — the node operator daemon. Derives a stable libp2p identity from a
//! **persisted** operator key, joins the Kademlia DHT over the production tcp+noise+yamux
//! transport, publishes its signed descriptor, and then runs the
//! [`weft_net::relay::RelayService`] event loop so it relays onion cells over the
//! `/weft/cell/1.0.0` transport and **egresses real internet traffic at the exit**
//! ([`weft_vpn::exit::InternetExit`]). To become a registered, reward-earning node, persist
//! the key (`WEFT_OPERATOR_KEY`) and register it on-chain from the written manifest (see
//! `services/registry-provision` `register-node`).

use std::env;
use std::path::Path;

use libp2p::kad;
use rand::rngs::StdRng;
use rand::SeedableRng;
use weft_net::discovery::{
    build_swarm, descriptor_signing_bytes, record_key, sign_descriptor, NodeDescriptor,
};
use weft_net::keys::WeftKeypair;

/// Load the operator key from `WEFT_OPERATOR_KEY` (96-byte hex: operator‖static‖onion seeds),
/// creating + saving a fresh one if the file is absent so the node keeps a **stable identity**
/// across restarts (required to be registered on-chain and accrue rewards). With the env unset
/// a throwaway key is used (fine for tests/demos, but the node can't be registered).
fn load_or_create_keypair() -> WeftKeypair {
    let Ok(path) = env::var("WEFT_OPERATOR_KEY") else {
        println!("[weft-node] no WEFT_OPERATOR_KEY — using an ephemeral key (not registerable)");
        return WeftKeypair::generate(&mut StdRng::from_entropy());
    };
    let p = Path::new(&path);
    if p.exists() {
        match std::fs::read_to_string(p)
            .ok()
            .and_then(|s| parse_key(s.trim()))
        {
            Some(kp) => {
                println!("[weft-node] loaded operator key from {path}");
                return kp;
            }
            None => eprintln!("[weft-node] WARNING: {path} unreadable/invalid — generating anew"),
        }
    }
    let kp = WeftKeypair::generate(&mut StdRng::from_entropy());
    let mut bytes = Vec::with_capacity(96);
    bytes.extend_from_slice(&kp.operator_seed());
    bytes.extend_from_slice(&kp.static_secret_bytes());
    bytes.extend_from_slice(&kp.onion_secret_bytes());
    match write_key(p, &hex::encode(bytes)) {
        Ok(()) => println!("[weft-node] generated + saved a new operator key → {path}"),
        Err(e) => eprintln!("[weft-node] WARNING: could not save key to {path}: {e}"),
    }
    kp
}

/// Parse a 96-byte hex key file (operator ed25519 seed ‖ x25519 static ‖ onion scalar).
fn parse_key(s: &str) -> Option<WeftKeypair> {
    let b = hex::decode(s).ok()?;
    if b.len() != 96 {
        return None;
    }
    let op: [u8; 32] = b[0..32].try_into().ok()?;
    let st: [u8; 32] = b[32..64].try_into().ok()?;
    let on: [u8; 32] = b[64..96].try_into().ok()?;
    Some(WeftKeypair::from_bytes(&op, &st, &on))
}

/// Write the key file, restricting it to owner-only on unix (best-effort).
fn write_key(p: &Path, hexs: &str) -> std::io::Result<()> {
    std::fs::write(p, hexs)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

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

    let kp = load_or_create_keypair();
    println!(
        "[weft-node] operator {}",
        hex::encode(kp.operator_pubkey())
    );

    // Build the swarm with the operator-derived libp2p identity, listen, and join the DHT.
    let mut swarm = build_swarm(kp.libp2p_keypair(), false)?;
    swarm.listen_on(listen.parse()?)?;
    swarm.behaviour_mut().kad.set_mode(Some(kad::Mode::Server));

    // NAT traversal: a node behind NAT (most home users) isn't directly reachable. If
    // public relay nodes are configured in WEFT_RELAYS (comma-separated multiaddrs like
    // /ip4/<ip>/tcp/<port>/p2p/<peer-id>), reserve a slot through each so peers can reach
    // this node via the relay; `dcutr` then upgrades that to a direct hole-punched link.
    // Public nodes need no relays — they listen directly and relay for others automatically.
    if let Ok(relays) = env::var("WEFT_RELAYS") {
        use libp2p::multiaddr::Protocol;
        for entry in relays.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match entry.parse::<libp2p::Multiaddr>() {
                Ok(addr) => {
                    if let Some(Protocol::P2p(peer)) = addr.iter().last() {
                        swarm.behaviour_mut().kad.add_address(&peer, addr.clone());
                    }
                    let _ = swarm.dial(addr.clone());
                    match swarm.listen_on(addr.with(Protocol::P2pCircuit)) {
                        Ok(_) => println!("[weft-node] reserving a relay slot via {entry}"),
                        Err(e) => eprintln!("[weft-node] relay reservation failed for {entry}: {e}"),
                    }
                }
                Err(e) => eprintln!("[weft-node] bad WEFT_RELAYS entry '{entry}': {e}"),
            }
        }
    }

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
        let endpoint_hash = hex::encode(weft_net::discovery::endpoint_commitment_parts(
            &kp.operator_pubkey(),
            node_id,
            &kp.static_public(),
            &kp.onion_public(),
            caps,
            geo,
        ));
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
            endpoint_hash,
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
    let exit = std::sync::Arc::new(weft_vpn::exit::InternetExit::new(policy));
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
