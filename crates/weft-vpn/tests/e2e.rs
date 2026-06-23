//! End-to-end VPN datapath test (no root): a real HTTP request is tunnelled through a real
//! 3-hop onion circuit (client → relay → relay → exit over libp2p MemoryTransport) and the
//! **exit dials a real local TCP origin**, proving traffic actually egresses. Covers both
//! the direct engine API and the SOCKS5 front-end.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use rand::rngs::StdRng;
use rand::SeedableRng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use weft_net::discovery::{build_swarm, routing_addr, WeftBehaviour};
use weft_net::exit::EchoExit;
use weft_net::keys::WeftKeypair;
use weft_net::node::Relay;
use weft_net::selection::NodeRecord;
use weft_primitives::capability;
use weft_vpn::client_engine::{ClientEngine, DialInfo};
use weft_vpn::exit::{EgressPolicy, InternetExit};

use weft_net::relay::{Clock, RelayService};

struct NodeInfo {
    kp: WeftKeypair,
    node_id: u64,
    record: NodeRecord,
    peer_id: PeerId,
}

/// 3 nodes: two relays + one EXIT-capable node (which gets the real internet exit).
fn make_nodes() -> Vec<NodeInfo> {
    let mut rng = StdRng::seed_from_u64(0x5ce1_2024);
    (0..3u64)
        .map(|i| {
            let kp = WeftKeypair::generate(&mut rng);
            let peer_id = kp.libp2p_keypair().public().to_peer_id();
            let caps = if i == 2 {
                capability::WIREGUARD | capability::EXIT
            } else {
                capability::WIREGUARD | capability::RELAY
            };
            let record = NodeRecord {
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
            NodeInfo {
                kp,
                node_id: i,
                record,
                peer_id,
            }
        })
        .collect()
}

async fn bind(kp: &WeftKeypair, listen: Multiaddr) -> (Swarm<WeftBehaviour>, Multiaddr) {
    let mut swarm = build_swarm(kp.libp2p_keypair(), true).unwrap();
    swarm.listen_on(listen).unwrap();
    let addr = loop {
        if let SwarmEvent::NewListenAddr { address, .. } = swarm.select_next_some().await {
            break address;
        }
    };
    (swarm, addr)
}

/// A tiny HTTP origin: respond with a fixed body then close.
async fn spawn_origin(body: &'static str) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf).await; // consume the request line/headers
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes()).await;
                let _ = s.flush().await;
            });
        }
    });
    addr
}

/// Stand up the 3-node circuit + a client engine over MemoryTransport. Returns the engine
/// and the allowlisted origin the exit may dial.
async fn setup(base: usize, origin_ip: std::net::IpAddr) -> ClientEngine {
    let nodes = make_nodes();

    // Phase 1: bind each node, learn (peer, addr, routing_addr).
    let mut swarms = Vec::new();
    let mut routes: Vec<(PeerId, Multiaddr, [u8; 32])> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let listen: Multiaddr = format!("/memory/{}", base + i).parse().unwrap();
        let (swarm, addr) = bind(&n.kp, listen).await;
        routes.push((n.peer_id, addr.clone(), n.record.addr));
        swarms.push(swarm);
    }

    // Phase 2: spawn each relay. Node 2 is the EXIT → real InternetExit; others echo (never
    // reached on the exit path). All get the full address book so forwards resolve.
    for (i, (n, swarm)) in nodes.iter().zip(swarms.into_iter()).enumerate() {
        let relay = Relay::new(n.kp.operator_pubkey(), n.node_id, n.kp.onion_secret(), 600);
        let exit: Box<dyn weft_net::exit::Exit> = if i == 2 {
            Box::new(InternetExit::new(EgressPolicy::allowlist(vec![origin_ip])))
        } else {
            Box::new(EchoExit)
        };
        let mut service = RelayService::new(swarm, relay, Clock::Fixed(700), exit);
        for (peer, addr, raddr) in &routes {
            service.add_route(*raddr, *peer, addr.clone());
        }
        tokio::spawn(async move {
            loop {
                service.step().await;
            }
        });
    }

    // The client directory + first-hop dialing map.
    let directory: Vec<NodeRecord> = nodes.iter().map(|n| n.record.clone()).collect();
    let mut dial: DialInfo = HashMap::new();
    for (i, (peer, addr, raddr)) in routes.iter().enumerate() {
        let _ = i;
        dial.insert(*raddr, (*peer, addr.clone()));
    }

    let mut rng = StdRng::seed_from_u64(7);
    let client_kp = WeftKeypair::generate(&mut rng);
    let listen: Multiaddr = format!("/memory/{}", base + 99).parse().unwrap();
    ClientEngine::spawn(&client_kp, true, listen, directory, dial, 3, 0)
        .await
        .unwrap()
}

#[tokio::test]
async fn http_tunnels_through_the_circuit_to_a_real_origin() {
    let origin = spawn_origin("hello-from-weft-exit").await;
    let engine = setup(20_000, origin.ip()).await;

    // Drive the engine directly: the "app side" writes an HTTP request and reads the body.
    let (mut app, tunnel_side) = tokio::io::duplex(64 * 1024);
    let h = tokio::spawn(async move { engine.tunnel(origin, 42, tunnel_side, false).await });

    app.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();

    let mut got = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        app.read_to_end(&mut got),
    )
    .await
    .expect("tunnel timed out")
    .unwrap();
    let text = String::from_utf8_lossy(&got);
    assert!(text.contains("200 OK"), "no status line: {text}");
    assert!(text.contains("hello-from-weft-exit"), "no body: {text}");
    let _ = h.await;
}

#[tokio::test]
async fn http_tunnels_through_the_socks5_proxy() {
    let origin = spawn_origin("hello-via-socks").await;
    let engine = Arc::new(setup(21_000, origin.ip()).await);
    let (socks, _socks_task) = weft_vpn::socks::serve(engine, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();

    // Speak SOCKS5 to our proxy, CONNECT to the origin, then do HTTP.
    let mut s = TcpStream::connect(socks).await.unwrap();
    s.write_all(&[0x05, 0x01, 0x00]).await.unwrap(); // greeting: 1 method, no-auth
    let mut g = [0u8; 2];
    s.read_exact(&mut g).await.unwrap();
    assert_eq!(g, [0x05, 0x00]);

    // CONNECT to 127.0.0.1:<origin port> (ATYP=IPv4).
    let ip = match origin.ip() {
        std::net::IpAddr::V4(v4) => v4.octets(),
        _ => panic!("origin is v4 in this test"),
    };
    let mut req = vec![0x05, 0x01, 0x00, 0x01];
    req.extend_from_slice(&ip);
    req.extend_from_slice(&origin.port().to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut reply = [0u8; 10];
    s.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0x00, "socks connect failed");

    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut got = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(20), s.read_to_end(&mut got))
        .await
        .expect("socks tunnel timed out")
        .unwrap();
    let text = String::from_utf8_lossy(&got);
    assert!(text.contains("hello-via-socks"), "no body: {text}");
}
