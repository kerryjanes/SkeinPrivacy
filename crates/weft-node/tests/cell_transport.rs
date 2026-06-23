//! Integration tests for the libp2p cell-transport (M7): real onion cells flowing
//! client → relay → exit → (reply) → client over libp2p request/response, in-process
//! over MemoryTransport (deterministic, CI) and over localhost TCP. Proves delivery,
//! recipient-hiding via addressing, per-hop metering, dual-signed receipts, and the
//! failure cases (a tampered cell).

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use rand::rngs::StdRng;
use rand::SeedableRng;
use weft_net::cell_transport::{err, CellResponse};
use weft_net::client;
use weft_net::discovery::{build_swarm, routing_addr, WeftBehaviour};
use weft_net::keys::WeftKeypair;
use weft_net::node::Relay;
use weft_net::receipt::ReceiptCore;
use weft_net::selection::NodeRecord;

#[path = "../src/relay_service.rs"]
mod relay_service;
use relay_service::{send_cell, Clock, ExitHandler, RelayService};

const DEST: [u8; 32] = [0xde; 32];

struct NodeInfo {
    kp: WeftKeypair,
    node_id: u64,
    record: NodeRecord,
    peer_id: PeerId,
}

fn make_nodes(n: usize) -> Vec<NodeInfo> {
    let mut rng = StdRng::seed_from_u64(0x5ce1_7777);
    (0..n)
        .map(|i| {
            let kp = WeftKeypair::generate(&mut rng);
            let node_id = i as u64;
            let peer_id = kp.libp2p_keypair().public().to_peer_id();
            let record = NodeRecord {
                operator: kp.operator_pubkey(),
                node_id,
                onion_pub: kp.onion_public(),
                static_pub: kp.static_public(),
                // routing addr = DHT record key, so relays resolve the next hop.
                addr: routing_addr(&kp.operator_pubkey(), node_id),
                geo: 100,
                capabilities: weft_primitives::capability::WIREGUARD
                    | weft_primitives::capability::RELAY
                    | weft_primitives::capability::EXIT,
                availability: 90,
                reputation_bps: 10_000,
            };
            NodeInfo {
                kp,
                node_id,
                record,
                peer_id,
            }
        })
        .collect()
}

/// Bind a swarm to `listen` and drive it until it reports its bound address.
async fn bind(
    kp: &WeftKeypair,
    memory: bool,
    listen: Multiaddr,
) -> (Swarm<WeftBehaviour>, Multiaddr) {
    let mut swarm = build_swarm(kp.libp2p_keypair(), memory).unwrap();
    swarm.listen_on(listen).unwrap();
    let addr = loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            break address;
        }
    };
    (swarm, addr)
}

/// A spawned relay's shared metering snapshot.
struct RelayHandle {
    metered: Arc<Mutex<u64>>,
    clients: Arc<Mutex<usize>>,
}

/// Echo exit: reply with `ack:` + the delivered payload (the trait impl in `weft-net`).
fn echo_exit() -> ExitHandler {
    Box::new(weft_net::exit::EchoExit)
}

/// Construct + spawn a relay service (already-bound swarm), seeding the full circuit
/// address book so peeled `next_addr`s resolve without a DHT round-trip.
fn spawn_relay(
    swarm: Swarm<WeftBehaviour>,
    kp: &WeftKeypair,
    node_id: u64,
    routes: &[(PeerId, Multiaddr, [u8; 32])],
) -> RelayHandle {
    let relay = Relay::new(kp.operator_pubkey(), node_id, kp.onion_secret(), 600);
    let mut service = RelayService::new(swarm, relay, Clock::Fixed(700), echo_exit());
    for (peer, addr, raddr) in routes {
        service.add_route(*raddr, *peer, addr.clone());
    }
    let metered = Arc::new(Mutex::new(0u64));
    let clients = Arc::new(Mutex::new(0usize));
    let (m, c) = (metered.clone(), clients.clone());
    tokio::spawn(async move {
        loop {
            service.step().await;
            *m.lock().unwrap() = service.relay().metered_total();
            *c.lock().unwrap() = service.relay().client_count();
        }
    });
    RelayHandle { metered, clients }
}

struct Circuit {
    client_kp: WeftKeypair,
    client_swarm: Swarm<WeftBehaviour>,
    first_peer: PeerId,
    first_addr: Multiaddr,
    path: Vec<NodeRecord>,
    cell: weft_net::sphinx::Cell,
    circuit: client::Circuit,
    handles: Vec<RelayHandle>,
    nodes: Vec<NodeInfo>,
}

/// Stand up a 3-relay circuit + a client over the chosen transport.
async fn setup(memory: bool, base_port: u16, payload: &[u8]) -> Circuit {
    setup_with_down(memory, base_port, payload, &[]).await
}

/// Like [`setup`], but the relays at the indices in `down` are bound (so their addresses
/// seed the circuit) then dropped without spawning — simulating an unreachable hop.
async fn setup_with_down(memory: bool, base_port: u16, payload: &[u8], down: &[usize]) -> Circuit {
    let nodes = make_nodes(3);
    let listen = |i: usize| -> Multiaddr {
        if memory {
            format!("/memory/{}", base_port as usize + i)
                .parse()
                .unwrap()
        } else {
            "/ip4/127.0.0.1/tcp/0".parse().unwrap()
        }
    };

    // Phase 1: bind every relay to learn its (peer_id, addr, routing_addr).
    let mut swarms = Vec::new();
    let mut routes: Vec<(PeerId, Multiaddr, [u8; 32])> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let (swarm, addr) = bind(&n.kp, memory, listen(i)).await;
        routes.push((n.peer_id, addr.clone(), n.record.addr));
        swarms.push(swarm);
    }

    // Phase 2: spawn each relay with the full circuit address book; drop the down ones.
    let mut handles = Vec::new();
    for (i, (n, swarm)) in nodes.iter().zip(swarms.into_iter()).enumerate() {
        if down.contains(&i) {
            drop(swarm); // closes its listener → downstream dials fail fast
            handles.push(RelayHandle {
                metered: Arc::new(Mutex::new(0)),
                clients: Arc::new(Mutex::new(0)),
            });
            continue;
        }
        handles.push(spawn_relay(swarm, &n.kp, n.node_id, &routes));
    }

    // Client: build a 3-hop onion over the directory (addr = routing_addr already).
    let mut rng = StdRng::seed_from_u64(99);
    let path: Vec<NodeRecord> = nodes.iter().map(|n| n.record.clone()).collect();
    let client_kp = WeftKeypair::generate(&mut rng);
    let (cell, circuit) = client::build_circuit(&mut rng, &path, DEST, payload).unwrap();

    let (client_swarm, _caddr) = bind(&client_kp, memory, listen(99)).await;
    let first_peer = routes[0].0;
    let first_addr = routes[0].1.clone();

    Circuit {
        client_kp,
        client_swarm,
        first_peer,
        first_addr,
        path,
        cell,
        circuit,
        handles,
        nodes,
    }
}

async fn happy_path(memory: bool, base_port: u16) {
    let payload = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
    let mut c = setup(memory, base_port, payload).await;

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        send_cell(
            &mut c.client_swarm,
            c.first_peer,
            c.first_addr.clone(),
            c.cell.clone(),
        ),
    )
    .await
    .expect("send_cell timed out");

    // the exit echoed our payload back through the reverse path.
    let sealed = match resp {
        CellResponse::Reply(b) => b,
        CellResponse::Err(e) => panic!("relay returned error {e}"),
    };
    let reply = client::open_reply(&c.circuit, &sealed).unwrap();
    let mut expected = b"ack:".to_vec();
    expected.extend_from_slice(payload);
    assert_eq!(reply, expected);

    // give the spawned relays a tick to publish their meter snapshots.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let wire = c.cell.wire_len() as u64;
    for h in &c.handles {
        assert_eq!(
            *h.metered.lock().unwrap(),
            wire,
            "each relay meters one cell"
        );
        assert_eq!(
            *h.clients.lock().unwrap(),
            1,
            "each relay sees one upstream"
        );
    }

    // a relay can dual-sign a receipt for the metered traffic (the test plays both keys).
    let core = ReceiptCore {
        client: c.client_kp.operator_pubkey(),
        operator: c.nodes[0].kp.operator_pubkey(),
        node_id: c.nodes[0].node_id,
        bytes: wire,
        window_start: 600,
        window_end: 1200,
        nonce: 1,
    };
    let cs = core.sign_client(&c.client_kp);
    let rs = core.sign_relay(&c.nodes[0].kp);
    assert!(core.verify(&cs, &rs));

    // recipient-hidden: no middle hop's onion addr equals the destination.
    for (i, hop) in c.path.iter().enumerate() {
        if i + 1 < c.path.len() {
            assert_ne!(hop.addr, DEST);
        }
    }
}

#[tokio::test]
async fn cell_roundtrips_over_memory_transport() {
    happy_path(true, 9100).await;
}

#[tokio::test]
async fn cell_roundtrips_over_localhost_tcp() {
    happy_path(false, 0).await;
}

#[tokio::test]
async fn tampered_cell_is_rejected() {
    let mut c = setup(true, 9200, b"x").await;
    c.cell.gamma[0] ^= 1; // corrupt the first hop's header MAC
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        send_cell(
            &mut c.client_swarm,
            c.first_peer,
            c.first_addr.clone(),
            c.cell.clone(),
        ),
    )
    .await
    .expect("send_cell timed out");
    assert_eq!(resp, CellResponse::Err(err::PEEL));
    // a dropped cell is never metered.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(*c.handles[0].metered.lock().unwrap(), 0);
}

#[tokio::test]
async fn downed_middle_hop_propagates_error() {
    // hop 1 (the middle relay) is unreachable; the first relay's forward fails and it
    // answers the client with a downstream error rather than hanging.
    let mut c = setup_with_down(true, 9300, b"x", &[1]).await;
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        send_cell(
            &mut c.client_swarm,
            c.first_peer,
            c.first_addr.clone(),
            c.cell.clone(),
        ),
    )
    .await
    .expect("send_cell timed out");
    assert_eq!(resp, CellResponse::Err(err::DOWNSTREAM));
    // the first relay still did real work (peeled + forwarded) so it metered its cell.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(
        *c.handles[0].metered.lock().unwrap(),
        c.cell.wire_len() as u64
    );
}
