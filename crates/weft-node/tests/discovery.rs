//! Two daemons discover each other over a real libp2p Kademlia DHT: the server
//! publishes its descriptor, the client resolves and validates it. Runs over the
//! socket-free MemoryTransport so it is deterministic in CI; the production path uses
//! the same code with tcp+noise+yamux.

use rand::rngs::StdRng;
use rand::SeedableRng;
use weft_net::keys::WeftKeypair;

#[path = "../src/daemon.rs"]
mod daemon;
use daemon::Daemon;

#[tokio::test]
async fn two_daemons_discover_via_dht() {
    let mut rng = StdRng::seed_from_u64(7);
    let server_kp = WeftKeypair::generate(&mut rng);
    let server_op = server_kp.operator_pubkey();
    let caps = weft_primitives::capability::WIREGUARD | weft_primitives::capability::RELAY;

    let mut server = Daemon::new(
        server_kp,
        1,
        100,
        caps,
        "/memory/7001".parse().unwrap(),
        true,
    )
    .unwrap();
    server.listen().unwrap();
    server.publish(1_700).unwrap();
    let server_peer = server.peer_id();

    let client_kp = WeftKeypair::generate(&mut rng);
    let mut client = Daemon::new(
        client_kp,
        2,
        100,
        caps,
        "/memory/7002".parse().unwrap(),
        true,
    )
    .unwrap();
    client.bootstrap(&server_peer, "/memory/7001".parse().unwrap());

    // drive the server in the background while the client resolves.
    let resolved = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            tokio::select! {
                _ = server.next_event() => {}
                r = client.resolve(&server_op, 1) => return r.unwrap(),
            }
        }
    })
    .await
    .expect("discovery timed out");

    assert_eq!(resolved.operator, server_op);
    assert_eq!(resolved.node_id, 1);
    assert_eq!(resolved.multiaddr, "/memory/7001");
}
