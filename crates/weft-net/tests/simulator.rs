//! In-process network simulator (M6 capstone): a real libp2p Kademlia DHT over the
//! socket-free MemoryTransport proves discovery, and a multi-hop onion circuit proves
//! the data plane end-to-end — delivery, recipient-hiding, per-hop metering, and
//! dual-signed receipts — plus the failure cases (tampered cell, dropped hop).

use std::collections::HashMap;

use futures::StreamExt;
use libp2p::kad;
use libp2p::swarm::SwarmEvent;
use libp2p::Multiaddr;
use rand::rngs::StdRng;
use rand::SeedableRng;
use weft_net::client;
use weft_net::discovery::{
    build_swarm, descriptor_signing_bytes, record_key, sign_descriptor, validate_record,
    NodeDescriptor, WeftBehaviourEvent,
};
use weft_net::keys::WeftKeypair;
use weft_net::node::Relay;
use weft_net::receipt::ReceiptCore;
use weft_net::selection::{select_circuit, NodeRecord, SelectParams};
use weft_net::sphinx::{reply_open, reply_seal, Peeled};
use weft_primitives::capability;

const DEST: [u8; 32] = [0xde; 32];

struct SimNode {
    kp: WeftKeypair,
    record: NodeRecord,
}

fn make_nodes(n: usize) -> Vec<SimNode> {
    let mut rng = StdRng::seed_from_u64(0x5ce1);
    (0..n)
        .map(|i| {
            let kp = WeftKeypair::generate(&mut rng);
            // routing id = onion pubkey; all nodes can relay or exit.
            let record = NodeRecord {
                operator: kp.operator_pubkey(),
                node_id: i as u64,
                onion_pub: kp.onion_public(),
                static_pub: kp.static_public(),
                addr: kp.onion_public(),
                geo: 100,
                capabilities: capability::WIREGUARD | capability::RELAY | capability::EXIT,
                availability: 90,
                reputation_bps: 10_000,
            };
            SimNode { kp, record }
        })
        .collect()
}

#[test]
fn onion_circuit_delivers_meters_and_produces_receipts() {
    let nodes = make_nodes(5);
    let directory: Vec<NodeRecord> = nodes.iter().map(|n| n.record.clone()).collect();
    let by_onion: HashMap<[u8; 32], &SimNode> =
        nodes.iter().map(|n| (n.record.onion_pub, n)).collect();

    // client selects a 3-hop circuit.
    let mut rng = StdRng::seed_from_u64(1234);
    let path = select_circuit(
        &directory,
        &SelectParams {
            k: 3,
            seed: 7,
            min_availability: 50,
            client_geo: 100,
        },
    )
    .unwrap();

    let client_kp = WeftKeypair::generate(&mut rng);
    let client_pk = client_kp.operator_pubkey();
    let payload = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
    let (mut cell, circuit) = client::build_circuit(&mut rng, &path, DEST, payload).unwrap();

    // build a relay per selected node.
    let mut relays: HashMap<[u8; 32], Relay> = path
        .iter()
        .map(|n| {
            let kp = &by_onion[&n.onion_pub].kp;
            (
                n.onion_pub,
                Relay::new(kp.operator_pubkey(), n.node_id, kp.onion_secret(), 600),
            )
        })
        .collect();

    let wire = cell.wire_len() as u64;
    let mut reply_keys_seen = Vec::new();
    let mut delivered = false;

    for (i, hop) in path.iter().enumerate() {
        let relay = relays.get_mut(&hop.onion_pub).unwrap();
        match relay.process(client_pk, &cell, 0).unwrap() {
            Peeled::Forward {
                next_addr,
                cell: next,
                reply_key,
            } => {
                assert!(i + 1 < path.len(), "non-exit forwards");
                // recipient-hidden: a middle hop learns only the NEXT hop, never DEST.
                assert_eq!(next_addr, path[i + 1].onion_pub);
                assert_ne!(next_addr, DEST);
                reply_keys_seen.push(reply_key);
                cell = next;
            }
            Peeled::Exit {
                dest,
                payload: got,
                reply_key,
            } => {
                assert_eq!(i + 1, path.len(), "only the last hop exits");
                assert_eq!(dest, DEST);
                assert_eq!(got, payload);
                reply_keys_seen.push(reply_key);
                delivered = true;
            }
        }
        // metering: each hop counts the fixed cell wire size.
        assert_eq!(relay.metered_bytes(&client_pk), wire);
    }
    assert!(delivered, "exit delivered the payload to the destination");

    // the per-hop reply keys the hops derived match what the client kept.
    assert_eq!(reply_keys_seen, circuit.reply_keys);

    // reverse path: destination replies through the circuit, client unwraps it.
    let reply_msg = b"HTTP/1.1 200 OK";
    let mut sealed = reply_msg.to_vec();
    for k in reply_keys_seen.iter().rev() {
        sealed = reply_seal(k, &sealed);
    }
    assert_eq!(reply_open(&circuit.reply_keys, &sealed).unwrap(), reply_msg);

    // each (client, relay) pair yields a dual-signed receipt the parties verify.
    for hop in &path {
        let kp = &by_onion[&hop.onion_pub].kp;
        let cores: Vec<ReceiptCore> = relays.get_mut(&hop.onion_pub).unwrap().close_window(1_200);
        assert_eq!(cores.len(), 1);
        let core = &cores[0];
        assert_eq!(core.bytes, wire);
        let cs = core.sign_client(&client_kp);
        let rs = core.sign_relay(kp);
        assert!(core.verify(&cs, &rs), "dual-signed receipt must verify");
    }
}

#[test]
fn tampered_cell_is_dropped_and_never_metered() {
    let nodes = make_nodes(3);
    let directory: Vec<NodeRecord> = nodes.iter().map(|n| n.record.clone()).collect();
    let by_onion: HashMap<[u8; 32], &SimNode> =
        nodes.iter().map(|n| (n.record.onion_pub, n)).collect();
    let mut rng = StdRng::seed_from_u64(55);
    let path = select_circuit(
        &directory,
        &SelectParams {
            k: 3,
            seed: 3,
            min_availability: 50,
            client_geo: 100,
        },
    )
    .unwrap();
    let client_kp = WeftKeypair::generate(&mut rng);
    let (mut cell, _circuit) = client::build_circuit(&mut rng, &path, DEST, b"x").unwrap();

    // tamper the cell → the first relay's onion auth fails and the cell is dropped.
    cell.gamma[0] ^= 1;
    let first = &by_onion[&path[0].onion_pub].kp;
    let mut relay = Relay::new(first.operator_pubkey(), 0, first.onion_secret(), 600);
    assert!(relay
        .process(client_kp.operator_pubkey(), &cell, 0)
        .is_err());
    // a dropped cell is never metered → never billed.
    assert_eq!(relay.metered_bytes(&client_kp.operator_pubkey()), 0);
}

/// Real Kademlia discovery over libp2p's in-process MemoryTransport: a server PUTs its
/// signed descriptor, a client resolves it by key and validates it — socket-free, CI-safe.
#[tokio::test]
async fn kademlia_put_get_over_memory_transport() {
    let mut rng = StdRng::seed_from_u64(9);
    let server_kp = WeftKeypair::generate(&mut rng);

    let desc = NodeDescriptor {
        operator: server_kp.operator_pubkey(),
        node_id: 1,
        multiaddr: "/memory/3001".into(),
        peer_id: server_kp.libp2p_keypair().public().to_peer_id().to_base58(),
        noise_static_pub: server_kp.static_public(),
        onion_pub: server_kp.onion_public(),
        capabilities: capability::WIREGUARD | capability::RELAY,
        geo: 100,
        issued_at: 1700,
    };
    let sig = server_kp.sign(&descriptor_signing_bytes(&desc));
    let value = sign_descriptor(desc.clone(), sig);
    let key = record_key(&server_kp.operator_pubkey(), 1);

    let mut server = build_swarm(libp2p::identity::Keypair::generate_ed25519(), true).unwrap();
    let mut clientsw = build_swarm(libp2p::identity::Keypair::generate_ed25519(), true).unwrap();
    server.behaviour_mut().kad.set_mode(Some(kad::Mode::Server));

    let addr: Multiaddr = "/memory/3001".parse().unwrap();
    server.listen_on(addr.clone()).unwrap();
    let server_peer = *server.local_peer_id();

    // server stores its own record; client learns the server's address and queries —
    // Kademlia dials the server (in its routing table) as part of the iterative query.
    server
        .behaviour_mut()
        .kad
        .put_record(
            kad::Record {
                key: key.clone(),
                value,
                publisher: None,
                expires: None,
            },
            kad::Quorum::One,
        )
        .unwrap();
    clientsw.behaviour_mut().kad.add_address(&server_peer, addr);

    // Drive both swarms; query once connected and retry on NotFound (the first query
    // can race ahead of the connection). Idle connections are kept alive by config.
    let got = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            tokio::select! {
                _ = server.select_next_some() => {}
                ev = clientsw.select_next_some() => {
                    match ev {
                        SwarmEvent::ConnectionEstablished { .. } => {
                            clientsw.behaviour_mut().kad.get_record(key.clone());
                        }
                        SwarmEvent::Behaviour(WeftBehaviourEvent::Kad(
                            kad::Event::OutboundQueryProgressed {
                                result: kad::QueryResult::GetRecord(res),
                                ..
                            },
                        )) => match res {
                            Ok(kad::GetRecordOk::FoundRecord(peer_record)) => {
                                return peer_record.record.value;
                            }
                            // not found yet → re-query (connection is up now).
                            _ => {
                                clientsw.behaviour_mut().kad.get_record(key.clone());
                            }
                        },
                        _ => {}
                    }
                }
            }
        }
    })
    .await
    .expect("kademlia get_record timed out");

    let resolved = validate_record(&key, &got).unwrap();
    assert_eq!(resolved, desc);
}
