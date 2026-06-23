//! The VPN client engine. Owns a libp2p swarm (driven by a background task that
//! multiplexes many in-flight cells), a node directory, and the per-flow tunnel loop that
//! both front-ends (SOCKS, TUN) share. A flow = a duplex byte stream to some internet
//! `dst`; the engine builds a fresh 3–5 hop onion circuit for it and pumps bytes as
//! [`crate::stream`] frames, one onion cell per round-trip, until either side closes.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;

use futures::StreamExt;
use libp2p::request_response::{self, OutboundRequestId};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use rand::{rngs::StdRng, Rng, SeedableRng};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

use weft_net::cell_transport::{err, CellResponse};
use weft_net::client::{build_circuit, open_reply};
use weft_net::discovery::{build_swarm, WeftBehaviour, WeftBehaviourEvent};
use weft_net::keys::WeftKeypair;
use weft_net::selection::{select_circuit, NodeRecord, SelectParams};
use weft_net::sphinx::Cell;

use crate::stream::{ClientFrame, ExitFrame, MAX_DATA_CHUNK};

/// First-hop dialing info for a routing address: the libp2p peer + its endpoint.
pub type DialInfo = HashMap<[u8; 32], (PeerId, Multiaddr)>;

enum Cmd {
    Send {
        first_peer: PeerId,
        first_addr: Multiaddr,
        cell: Cell,
        reply: oneshot::Sender<CellResponse>,
    },
}

/// A cheap, cloneable handle to the swarm task: send one cell, await its sealed reply.
#[derive(Clone)]
pub struct ClientHandle {
    tx: mpsc::Sender<Cmd>,
}

impl ClientHandle {
    async fn round_trip(
        &self,
        first_peer: PeerId,
        first_addr: Multiaddr,
        cell: Cell,
    ) -> CellResponse {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(Cmd::Send {
                first_peer,
                first_addr,
                cell,
                reply,
            })
            .await
            .is_err()
        {
            return CellResponse::Err(err::DOWNSTREAM);
        }
        rx.await.unwrap_or(CellResponse::Err(err::DOWNSTREAM))
    }
}

/// Drive the client swarm: accept send commands, dispatch cell responses back to waiters.
async fn run_swarm(mut swarm: Swarm<WeftBehaviour>, mut rx: mpsc::Receiver<Cmd>) {
    let mut pending: HashMap<OutboundRequestId, oneshot::Sender<CellResponse>> = HashMap::new();
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(Cmd::Send { first_peer, first_addr, cell, reply }) => {
                    swarm.add_peer_address(first_peer, first_addr);
                    let id = swarm.behaviour_mut().cell.send_request(&first_peer, cell);
                    pending.insert(id, reply);
                }
                None => break, // all handles dropped → shut down
            },
            ev = swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(WeftBehaviourEvent::Cell(ev)) = ev {
                    match ev {
                        request_response::Event::Message {
                            message: request_response::Message::Response { request_id, response },
                            ..
                        } => {
                            if let Some(tx) = pending.remove(&request_id) {
                                let _ = tx.send(response);
                            }
                        }
                        request_response::Event::OutboundFailure { request_id, .. } => {
                            if let Some(tx) = pending.remove(&request_id) {
                                let _ = tx.send(CellResponse::Err(err::DOWNSTREAM));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
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
    handle: ClientHandle,
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
        let (tx, rx) = mpsc::channel(256);
        let swarm_task = tokio::spawn(run_swarm(swarm, rx));
        Ok(Self {
            handle: ClientHandle { tx },
            nodes,
            dial,
            hops: hops.clamp(2, 5),
            client_geo,
            counters: Counters::default(),
            swarm_task,
        })
    }

    /// A clone of the underlying swarm handle (for advanced callers).
    pub fn handle(&self) -> ClientHandle {
        self.handle.clone()
    }

    /// Live byte counters (clone is cheap; shares the same atomics).
    pub fn counters(&self) -> Counters {
        self.counters.clone()
    }

    /// One onion round-trip: wrap `payload` for `path` (exit selector = the exit's routing
    /// addr), send to the first hop, open the layered reply, decode the exit frame.
    async fn onion_round_trip(
        &self,
        rng: &mut StdRng,
        path: &[NodeRecord],
        exit_addr: [u8; 32],
        first_peer: PeerId,
        first_addr: &Multiaddr,
        payload: &[u8],
    ) -> io::Result<ExitFrame> {
        let (cell, circuit) = build_circuit(rng, path, exit_addr, payload)
            .map_err(|e| io::Error::other(format!("build_circuit: {e}")))?;
        let sealed = match self
            .handle
            .round_trip(first_peer, first_addr.clone(), cell)
            .await
        {
            CellResponse::Reply(b) => b,
            CellResponse::Err(code) => {
                return Err(io::Error::other(format!("circuit error {code}")))
            }
        };
        let opened =
            open_reply(&circuit, &sealed).map_err(|e| io::Error::other(format!("reply: {e}")))?;
        ExitFrame::decode(&opened).ok_or_else(|| io::Error::other("bad exit frame"))
    }

    /// Tunnel a duplex byte stream `io` (an accepted SOCKS/TUN flow) to `dst` through a fresh
    /// circuit until either side closes. `udp` selects TCP vs UDP egress at the exit.
    ///
    /// Full-duplex + pipelined: independent up- and down-pumps run concurrently. The up-pump
    /// sends `Data` cells (writes at the exit, one in order); the down-pump sends `Poll`
    /// cells (drains the exit's read side, one in order). Separating the directions keeps
    /// each ordered while overlapping them, and the exit's large read window per `Poll`
    /// keeps download throughput ≈ window / RTT.
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
        let (first_peer, first_addr) = self
            .dial
            .get(&first.addr)
            .cloned()
            .ok_or_else(|| io::Error::other("no dial info for first hop"))?;
        let exit_addr = path.last().expect("non-empty path").addr;
        let stream_id: u64 = rng.gen();

        // OPEN the stream at the exit.
        let open = ClientFrame::Open {
            stream_id,
            dst,
            udp,
        }
        .encode();
        if let ExitFrame::Err(code) = self
            .onion_round_trip(&mut rng, &path, exit_addr, first_peer, &first_addr, &open)
            .await?
        {
            return Err(io::Error::other(format!("exit open failed: {code}")));
        }

        let (mut rd, mut wr) = tokio::io::split(io);

        // Up-pump: app → exit. One `Data` cell at a time (ordered).
        let up = async {
            let mut rng = StdRng::seed_from_u64(seed ^ 0x5151_5151);
            let mut buf = vec![0u8; MAX_DATA_CHUNK];
            let mut seq: u32 = 0;
            loop {
                let n = rd.read(&mut buf).await?;
                if n == 0 {
                    break; // app closed its write side
                }
                self.counters
                    .up
                    .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                let payload = ClientFrame::Data {
                    stream_id,
                    seq,
                    data: buf[..n].to_vec(),
                }
                .encode();
                seq = seq.wrapping_add(1);
                match self
                    .onion_round_trip(
                        &mut rng,
                        &path,
                        exit_addr,
                        first_peer,
                        &first_addr,
                        &payload,
                    )
                    .await?
                {
                    ExitFrame::Err(_) | ExitFrame::Eof => break,
                    ExitFrame::Data(_) => {}
                }
            }
            io::Result::Ok(())
        };

        // Down-pump: exit → app. Continuously `Poll`; the exit's 25ms read deadline paces
        // idle polling, and a non-empty reply means more is ready (poll again immediately).
        let down = async {
            let mut rng = StdRng::seed_from_u64(seed ^ 0xACAC_ACAC);
            let poll = ClientFrame::Poll { stream_id }.encode();
            // Loops while the exit keeps returning Data; an Eof/Err reply ends it.
            while let ExitFrame::Data(d) = self
                .onion_round_trip(&mut rng, &path, exit_addr, first_peer, &first_addr, &poll)
                .await?
            {
                if !d.is_empty() {
                    self.counters
                        .down
                        .fetch_add(d.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    wr.write_all(&d).await?;
                }
            }
            wr.flush().await?;
            io::Result::Ok(())
        };

        // Whichever direction ends first finishes the flow; then close the exit stream.
        tokio::select! {
            r = up => r?,
            r = down => r?,
        }
        let close = ClientFrame::Close { stream_id }.encode();
        let _ = self
            .onion_round_trip(&mut rng, &path, exit_addr, first_peer, &first_addr, &close)
            .await;
        Ok(())
    }
}
