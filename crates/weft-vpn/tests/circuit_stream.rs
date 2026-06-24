//! Integration test for the streaming data plane (persistent full-duplex circuits). A real
//! 3-hop onion circuit is built ONCE, then many forward cells and pushed reverse cells flow
//! over persistent libp2p substreams (`/weft/circuit/1.0.0`) — client → relay → relay →
//! exit → real origin → (pushed) → client, in-process over MemoryTransport. Proves: the exit
//! PUSHES download bytes (no polling), one header carries many cells, the HTTP body
//! reassembles across multiple reverse cells, and every hop meters real wire bytes.

use std::sync::Arc;
use std::time::Duration;

use futures::{AsyncReadExt, StreamExt};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use rand::rngs::StdRng;
use rand::SeedableRng;

use weft_net::circuit::{read_frame, write_frame, Frame, CIRCUIT_PROTOCOL};
use weft_net::circuit_relay::CircuitRelayService;
use weft_net::client::build_stream_circuit;
use weft_net::discovery::{build_swarm, routing_addr, WeftBehaviour};
use weft_net::keys::WeftKeypair;
use weft_net::node::Relay;
use weft_net::relay::Clock;
use weft_net::selection::NodeRecord;

use weft_vpn::exit::{EgressPolicy, InternetExit};
use weft_vpn::stream::{ClientFrame, ExitFrame};

use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;

struct NodeInfo {
    kp: WeftKeypair,
    node_id: u64,
    record: NodeRecord,
    peer_id: PeerId,
}

fn make_nodes(n: usize) -> Vec<NodeInfo> {
    let mut rng = StdRng::seed_from_u64(0x5ce1_c1c1);
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

#[tokio::test]
async fn streaming_circuit_pushes_a_full_http_response() {
    // A local "internet" origin: read the request, then stream a body larger than one cell so
    // the response arrives as several pushed reverse cells.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = listener.local_addr().unwrap();
    let body: Vec<u8> = (0..40_000u32).map(|i| i as u8).collect();
    let body_for_origin = body.clone();
    tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 64];
        let _ = s.read(&mut buf).await;
        let mut resp = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        resp.extend_from_slice(&body_for_origin);
        s.write_all(&resp).await.unwrap();
    });

    let nodes = make_nodes(3);
    let listen = |i: usize| -> Multiaddr { format!("/memory/{}", 0xC1C0 + i).parse().unwrap() };

    // Bind every relay first to learn its (peer, addr, routing_addr).
    let mut swarms = Vec::new();
    let mut routes: Vec<([u8; 32], PeerId, Multiaddr)> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let (swarm, addr) = bind(&n.kp, listen(i)).await;
        routes.push((n.record.addr, n.peer_id, addr.clone()));
        swarms.push(swarm);
    }

    // Spawn each relay as a streaming circuit service; the exit hop dials the real origin.
    let mut relay_handles = Vec::new();
    for (n, swarm) in nodes.iter().zip(swarms.into_iter()) {
        let relay = Relay::new(n.kp.operator_pubkey(), n.node_id, n.kp.onion_secret(), 600);
        let exit = Arc::new(InternetExit::new(EgressPolicy::allowlist(vec![origin.ip()])));
        let mut service = CircuitRelayService::new(swarm, relay, Clock::Fixed(700), exit);
        for (raddr, peer, addr) in &routes {
            service.add_route(*raddr, *peer, addr.clone());
        }
        relay_handles.push(service.relay());
        tokio::spawn(service.run());
    }

    // Client: build the streaming circuit (one header) over the 3-hop path; exit dials origin.
    let mut rng = StdRng::seed_from_u64(7);
    let path: Vec<NodeRecord> = nodes.iter().map(|n| n.record.clone()).collect();
    let dest = routing_addr(&[0xab; 32], 4242); // exit dials `origin` (in the ClientFrame), not this
    let sc = build_stream_circuit(&mut rng, &path, dest).unwrap();

    let client_kp = WeftKeypair::generate(&mut rng);
    let (mut client_swarm, _caddr) = bind(&client_kp, "/memory/49999".parse().unwrap()).await;
    let first_peer = routes[0].1;
    let first_addr = routes[0].2.clone();
    client_swarm.add_peer_address(first_peer, first_addr);
    let mut control = client_swarm.behaviour().circuit.new_control();
    tokio::spawn(async move {
        loop {
            client_swarm.select_next_some().await;
        }
    });

    // Open the persistent circuit substream to the first hop (auto-dials).
    let stream = tokio::time::timeout(
        Duration::from_secs(20),
        control.open_stream(first_peer, CIRCUIT_PROTOCOL),
    )
    .await
    .expect("open_stream timed out")
    .expect("open_stream failed");
    let (mut r, mut w) = stream.split();

    // OPEN: the reused Sphinx header + a circuit id.
    write_frame(&mut w, &Frame::Open { circuit_id: [1u8; 16], header: sc.header.clone() })
        .await
        .unwrap();

    // FWD #0: open a TCP stream at the exit to the origin.
    let open = ClientFrame::Open { stream_id: 1, dst: origin, udp: false }.encode();
    write_frame(&mut w, &Frame::Fwd { seq: 0, delta: sc.seal(0, &open).unwrap() })
        .await
        .unwrap();

    // FWD #1: send the HTTP request (same header reused — only the seq advances).
    let req = ClientFrame::Data { stream_id: 1, seq: 0, data: b"GET / HTTP/1.0\r\n\r\n".to_vec() }
        .encode();
    write_frame(&mut w, &Frame::Fwd { seq: 1, delta: sc.seal(1, &req).unwrap() })
        .await
        .unwrap();

    // Receive pushed reverse cells until we have the whole body.
    let mut got = Vec::new();
    let deadline = Duration::from_secs(20);
    loop {
        let frame = tokio::time::timeout(deadline, read_frame(&mut r))
            .await
            .expect("reverse read timed out")
            .unwrap();
        match frame {
            Some(Frame::Rev { seq, sealed }) => {
                let opened = sc.open(seq, &sealed).unwrap();
                match ExitFrame::decode(&opened).unwrap() {
                    ExitFrame::Data(d) => got.extend_from_slice(&d),
                    ExitFrame::Eof => break,
                    ExitFrame::Err(e) => panic!("exit error {e}"),
                }
            }
            Some(_) => {}
            None => break,
        }
        if got.windows(4).any(|c| c == b"\r\n\r\n") && got.len() >= 18 + body.len() {
            break;
        }
    }

    let split = got
        .windows(4)
        .position(|c| c == b"\r\n\r\n")
        .expect("response has a header/body split");
    let header_txt = String::from_utf8_lossy(&got[..split]);
    assert!(header_txt.contains("200 OK"), "status line: {header_txt}");
    assert_eq!(&got[split + 4..], &body[..], "full body reassembled across reverse cells");

    // Every hop metered real wire bytes (header + forward + reverse cells).
    tokio::time::sleep(Duration::from_millis(100)).await;
    for (i, h) in relay_handles.iter().enumerate() {
        let metered = h.lock().unwrap().metered_total();
        assert!(metered > 0, "hop {i} should meter traffic, got {metered}");
    }
}
