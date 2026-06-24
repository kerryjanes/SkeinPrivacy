//! The VPN client engine. Owns a libp2p swarm (driven by a background task) plus a node
//! directory, and the per-flow tunnel loop that both front-ends (SOCKS, VLESS) share. A flow
//! = a duplex byte stream to some internet `dst`; the engine builds ONE 3–5 hop onion circuit
//! for it over a persistent full-duplex substream, then pumps bytes as [`crate::stream`]
//! frames — pipelined uploads + pushed downloads, no per-cell round-trip — until it closes.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::Ordering::Relaxed;

use futures::{AsyncReadExt as _, StreamExt};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use rand::{rngs::StdRng, Rng, SeedableRng};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use weft_net::circuit::{read_frame, write_frame, CircuitControl, Frame, CIRCUIT_PROTOCOL};
use weft_net::client::build_stream_circuit;
use weft_net::discovery::{build_swarm, WeftBehaviour};
use weft_net::keys::WeftKeypair;
use weft_net::selection::{select_circuit, NodeRecord, SelectParams};

use crate::stream::{ClientFrame, ExitFrame, MAX_DATA_CHUNK};

/// First-hop dialing info for a routing address: the libp2p peer + its endpoint.
pub type DialInfo = HashMap<[u8; 32], (PeerId, Multiaddr)>;

/// Drive the client swarm: dialing, connection upkeep, and the circuit-stream protocol all
/// run through `select_next_some`. Opening circuits goes through the cloneable
/// [`CircuitControl`] obtained before this task takes ownership of the swarm.
async fn run_swarm(mut swarm: Swarm<WeftBehaviour>) {
    loop {
        let _ = swarm.select_next_some().await;
    }
}

/// Live throughput counters (bytes the engine has tunnelled), shared with the UI.
#[derive(Clone, Default)]
pub struct Counters {
    pub up: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub down: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// The VPN client engine.
pub struct ClientEngine {
    control: CircuitControl,
    nodes: Vec<NodeRecord>,
    dial: DialInfo,
    hops: usize,
    client_geo: u32,
    counters: Counters,
    swarm_task: tokio::task::JoinHandle<()>,
}

impl Drop for ClientEngine {
    fn drop(&mut self) {
        self.swarm_task.abort();
    }
}

impl ClientEngine {
    /// Build the engine: bind a client swarm (identity from `kp`), spawn its driver task,
    /// and keep the node directory + first-hop dialing map for circuit construction.
    /// `listen` is the swarm's own listen address (ephemeral; outbound-only in practice).
    pub async fn spawn(
        kp: &WeftKeypair,
        memory: bool,
        listen: Multiaddr,
        nodes: Vec<NodeRecord>,
        dial: DialInfo,
        hops: usize,
        client_geo: u32,
    ) -> io::Result<Self> {
        let mut swarm = build_swarm(kp.libp2p_keypair(), memory)
            .map_err(|e| io::Error::other(format!("build_swarm: {e}")))?;
        swarm
            .listen_on(listen)
            .map_err(|e| io::Error::other(format!("listen: {e}")))?;
        // Drive until the listener is up so dialing works.
        loop {
            if let SwarmEvent::NewListenAddr { .. } = swarm.select_next_some().await {
                break;
            }
        }
        // Pre-seed every first-hop candidate's address so `open_stream` can auto-dial it.
        for (peer, addr) in dial.values() {
            swarm.add_peer_address(*peer, addr.clone());
        }
        let control = swarm.behaviour().circuit.new_control();
        let swarm_task = tokio::spawn(run_swarm(swarm));
        Ok(Self {
            control,
            nodes,
            dial,
            hops: hops.clamp(2, 5),
            client_geo,
            counters: Counters::default(),
            swarm_task,
        })
    }

    /// Connect to an external Weft network described by bootstrap [`crate::manifest`]s
    /// (real relay/exit processes over TCP), building the circuit directory + first-hop
    /// dialing map from them. This is the distributed path (vs the in-process localnet).
    pub async fn connect(
        client_kp: &WeftKeypair,
        manifests: &[crate::manifest::NodeManifest],
        hops: usize,
        client_geo: u32,
    ) -> io::Result<Self> {
        let mut directory = Vec::new();
        let mut dial = DialInfo::new();
        for m in manifests {
            if let (Some(rec), Some(peer)) = (m.record(), m.dial()) {
                dial.insert(rec.addr, peer);
                directory.push(rec);
            }
        }
        if directory.is_empty() {
            return Err(io::Error::other("no usable node manifests"));
        }
        Self::spawn(
            client_kp,
            false,
            "/ip4/0.0.0.0/tcp/0".parse().unwrap(),
            directory,
            dial,
            hops,
            client_geo,
        )
        .await
    }

    /// Live byte counters (clone is cheap; shares the same atomics).
    pub fn counters(&self) -> Counters {
        self.counters.clone()
    }

    /// Tunnel a duplex byte stream `io` (an accepted SOCKS/TUN flow) to `dst` through a fresh
    /// circuit until it closes. `udp` selects TCP vs UDP egress at the exit.
    ///
    /// The circuit is built ONCE (one Sphinx header) over a persistent full-duplex substream;
    /// then two pumps run concurrently. The up-pump pipelines `Data` cells (each sealed at a
    /// unique forward `seq`, no waiting for a reply); the down-pump receives the cells the exit
    /// PUSHES as the real connection produces bytes (no polling). The flow ends when the
    /// download side closes (server EOF / error); the upload side finishing (app half-close)
    /// does not cut an in-flight download.
    pub async fn tunnel<S>(&self, dst: SocketAddr, seed: u64, io: S, udp: bool) -> io::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let mut rng = StdRng::seed_from_u64(seed);
        let params = SelectParams {
            k: self.hops,
            seed: rng.gen(),
            min_availability: 0,
            client_geo: self.client_geo,
        };
        let path = select_circuit(&self.nodes, &params)
            .map_err(|e| io::Error::other(format!("select_circuit: {e}")))?;
        let first = &path[0];
        let (first_peer, _first_addr) = self
            .dial
            .get(&first.addr)
            .cloned()
            .ok_or_else(|| io::Error::other("no dial info for first hop"))?;
        let exit_addr = path.last().expect("non-empty path").addr;
        let stream_id: u64 = rng.gen();

        // Build the streaming circuit (one reusable header) and open the persistent substream
        // to the first hop (auto-dials using the pre-seeded address).
        let sc = std::sync::Arc::new(
            build_stream_circuit(&mut rng, &path, exit_addr)
                .map_err(|e| io::Error::other(format!("build_stream_circuit: {e}")))?,
        );
        let stream = self
            .control
            .clone()
            .open_stream(first_peer, CIRCUIT_PROTOCOL)
            .await
            .map_err(|e| io::Error::other(format!("open circuit: {e:?}")))?;
        let (mut sr, mut sw) = stream.split();

        // OPEN: the reused header + a fresh circuit id. FWD #0: open the exit's TCP/UDP stream.
        write_frame(
            &mut sw,
            &Frame::Open {
                circuit_id: rng.gen(),
                header: sc.header.clone(),
            },
        )
        .await?;
        let open = ClientFrame::Open { stream_id, dst, udp }.encode();
        let delta = sc
            .seal(0, &open)
            .map_err(|e| io::Error::other(format!("seal: {e}")))?;
        write_frame(&mut sw, &Frame::Fwd { seq: 0, delta }).await?;

        let (mut rd, mut wr) = tokio::io::split(io);

        // Up-pump: app → exit. Pipelined forward cells; forward `seq` strictly increases
        // (cell 0 was the OPEN above, so app data starts at 1).
        let counters = self.counters.clone();
        let sc_up = sc.clone();
        let up = async move {
            let sc = sc_up;
            let mut buf = vec![0u8; MAX_DATA_CHUNK];
            let mut fseq: u64 = 1;
            let mut dseq: u32 = 0;
            loop {
                let n = rd.read(&mut buf).await?;
                if n == 0 {
                    break; // app closed its write side
                }
                counters.up.fetch_add(n as u64, Relaxed);
                let payload = ClientFrame::Data {
                    stream_id,
                    seq: dseq,
                    data: buf[..n].to_vec(),
                }
                .encode();
                dseq = dseq.wrapping_add(1);
                let delta = sc
                    .seal(fseq, &payload)
                    .map_err(|e| io::Error::other(format!("seal: {e}")))?;
                write_frame(&mut sw, &Frame::Fwd { seq: fseq, delta }).await?;
                fseq += 1;
            }
            io::Result::Ok(())
        };

        // Down-pump: exit → app. Receive pushed reverse cells, open each at its reverse `seq`.
        let counters = self.counters.clone();
        let down = async move {
            loop {
                match read_frame(&mut sr).await? {
                    Some(Frame::Rev { seq, sealed }) => {
                        let opened = sc
                            .open(seq, &sealed)
                            .map_err(|e| io::Error::other(format!("reply open: {e}")))?;
                        match ExitFrame::decode(&opened) {
                            Some(ExitFrame::Data(d)) => {
                                if !d.is_empty() {
                                    counters.down.fetch_add(d.len() as u64, Relaxed);
                                    wr.write_all(&d).await?;
                                }
                            }
                            Some(ExitFrame::Eof) | None => break,
                            Some(ExitFrame::Err(_)) => break,
                        }
                    }
                    Some(Frame::Close) | None => break,
                    Some(_) => {}
                }
            }
            wr.flush().await?;
            io::Result::Ok(())
        };

        // Full-duplex: the flow ends when EITHER pump ends — the download side on server EOF,
        // the upload side when the app closes its write half (a real client keeps that half
        // open while awaiting the response, so this only fires on a genuine disconnect). On
        // return, dropping the substream halves tears the circuit down at every hop.
        tokio::select! {
            r = up => r,
            r = down => r,
        }
    }
}
