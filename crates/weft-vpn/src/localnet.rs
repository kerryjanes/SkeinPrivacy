//! A self-contained, in-process Weft circuit for demos and the desktop app's "works on my
//! machine" mode: `hops`+slack nodes (relays + one real-egress exit) over libp2p
//! MemoryTransport, plus a [`ClientEngine`] wired to them. Lets the VPN tunnel real traffic
//! out this machine's exit without any deployed infrastructure. Production connects to the
//! live DHT network instead (same engine, real node directory).

use std::collections::HashMap;
use std::io;

use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use rand::rngs::StdRng;
use rand::SeedableRng;

use weft_net::discovery::{build_swarm, routing_addr, WeftBehaviour};
use weft_net::exit::{EchoExit, Exit};
use weft_net::keys::WeftKeypair;
use weft_net::node::Relay;
use weft_net::relay::{Clock, RelayService};
use weft_net::selection::NodeRecord;
use weft_primitives::capability;

use crate::client_engine::{ClientEngine, DialInfo};
use crate::exit::{EgressPolicy, InternetExit};

/// Bind a node over the real TCP transport (real noise + yamux), returning its swarm and
/// the actual bound address.
async fn bind(kp: &WeftKeypair) -> io::Result<(Swarm<WeftBehaviour>, Multiaddr)> {
    let mut swarm =
        build_swarm(kp.libp2p_keypair(), false).map_err(|e| io::Error::other(e.to_string()))?;
    swarm
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .map_err(|e| io::Error::other(e.to_string()))?;
    let addr = loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            break address;
        }
    };
    Ok((swarm, addr))
}

/// A running in-process circuit: the client engine plus the relay task handles, which are
/// aborted on drop so a `disconnect` tears the whole local network down cleanly.
pub struct LocalNet {
    pub engine: std::sync::Arc<ClientEngine>,
    relays: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for LocalNet {
    fn drop(&mut self) {
        for h in &self.relays {
            h.abort();
        }
    }
}

/// Spawn an in-process circuit and return it wired to a client engine. The last node is the
/// real internet exit (governed by `exit_policy`); the rest are relays. `exit_bind_if` pins
/// the exit's egress to an interface index (macOS `IP_BOUND_IF`) so that, when this runs on
/// the same host as a TUN client, the exit's own connections bypass the tunnel routes.
pub async fn spawn(
    hops: usize,
    exit_policy: EgressPolicy,
    base: usize,
    exit_bind_if: Option<u32>,
) -> io::Result<LocalNet> {
    let hops = hops.clamp(2, 5);
    let n = hops + 2; // a little path diversity beyond the minimum
    let mut rng = StdRng::seed_from_u64(0xc0ffee ^ base as u64);

    struct N {
        kp: WeftKeypair,
        id: u64,
        rec: NodeRecord,
        peer: PeerId,
    }
    let nodes: Vec<N> = (0..n as u64)
        .map(|i| {
            let kp = WeftKeypair::generate(&mut rng);
            let peer = kp.libp2p_keypair().public().to_peer_id();
            let caps = if i == n as u64 - 1 {
                capability::WIREGUARD | capability::EXIT
            } else {
                capability::WIREGUARD | capability::RELAY
            };
            let rec = NodeRecord {
                operator: kp.operator_pubkey(),
                node_id: i,
                onion_pub: kp.onion_public(),
                static_pub: kp.static_public(),
                addr: routing_addr(&kp.operator_pubkey(), i),
                geo: 100,
                capabilities: caps,
                availability: 95,
                reputation_bps: 10_000,
            };
            N {
                kp,
                id: i,
                rec,
                peer,
            }
        })
        .collect();

    // Bind every node over real TCP, learn its bound address.
    let mut swarms = Vec::new();
    let mut routes: Vec<(PeerId, Multiaddr, [u8; 32])> = Vec::new();
    for nd in nodes.iter() {
        let (sw, addr) = bind(&nd.kp).await?;
        routes.push((nd.peer, addr.clone(), nd.rec.addr));
        swarms.push(sw);
    }

    // Spawn each relay; the last is the real internet exit.
    let last = nodes.len() - 1;
    let mut relays = Vec::new();
    for (i, (nd, sw)) in nodes.iter().zip(swarms.into_iter()).enumerate() {
        let relay = Relay::new(nd.kp.operator_pubkey(), nd.id, nd.kp.onion_secret(), 0);
        let exit: Box<dyn Exit> = if i == last {
            Box::new(InternetExit::new(exit_policy.clone()).with_bind_interface(exit_bind_if))
        } else {
            Box::new(EchoExit)
        };
        let mut svc = RelayService::new(sw, relay, Clock::System, exit);
        for (peer, addr, raddr) in &routes {
            svc.add_route(*raddr, *peer, addr.clone());
        }
        relays.push(tokio::spawn(async move {
            loop {
                svc.step().await;
            }
        }));
    }

    let directory: Vec<NodeRecord> = nodes.iter().map(|nd| nd.rec.clone()).collect();
    let mut dial: DialInfo = HashMap::new();
    for (peer, addr, raddr) in &routes {
        dial.insert(*raddr, (*peer, addr.clone()));
    }
    let client_kp = WeftKeypair::generate(&mut rng);
    let listen: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let engine = ClientEngine::spawn(&client_kp, false, listen, directory, dial, hops, 0).await?;
    Ok(LocalNet {
        engine: std::sync::Arc::new(engine),
        relays,
    })
}
