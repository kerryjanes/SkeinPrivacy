//! The VPN client engine. Owns a libp2p swarm (driven by a background task that
//! multiplexes many in-flight cells), a node directory, and the per-flow tunnel loop that
//! both front-ends (SOCKS, TUN) share. A flow = a duplex byte stream to some internet
//! `dst`; the engine builds a fresh 3–5 hop onion circuit for it and pumps bytes as
//! [`crate::stream`] frames, one onion cell per round-trip, until either side closes.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

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

/// How long a flow waits for app-side bytes before sending a `Poll` to drain the
/// server→client direction. Lower = snappier downloads, more cells; higher = fewer cells.
const IDLE_POLL: Duration = Duration::from_millis(8);

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

    /// Tunnel a duplex byte stream `io` (an accepted SOCKS/TUN connection) to `dst` through
    /// a fresh circuit until either side closes. This is the core both front-ends call.
    pub async fn tunnel<S>(&self, dst: SocketAddr, seed: u64, mut io: S) -> io::Result<()>
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
        let open = ClientFrame::Open { stream_id, dst }.encode();
        if let ExitFrame::Err(code) = self
            .onion_round_trip(&mut rng, &path, exit_addr, first_peer, &first_addr, &open)
            .await?
        {
            return Err(io::Error::other(format!("exit open failed: {code}")));
        }

        // Pump bytes both ways, one cell per round-trip.
        let mut seq: u32 = 0;
        let mut buf = vec![0u8; MAX_DATA_CHUNK];
        loop {
            let frame = match tokio::time::timeout(IDLE_POLL, io.read(&mut buf)).await {
                Ok(Ok(0)) => {
                    // App side closed: tell the exit, then finish.
                    let close = ClientFrame::Close { stream_id }.encode();
                    let _ = self
                        .onion_round_trip(
                            &mut rng,
                            &path,
                            exit_addr,
                            first_peer,
                            &first_addr,
                            &close,
                        )
                        .await;
                    return Ok(());
                }
                Ok(Ok(n)) => {
                    self.counters
                        .up
                        .fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
                    let f = ClientFrame::Data {
                        stream_id,
                        seq,
                        data: buf[..n].to_vec(),
                    };
                    seq = seq.wrapping_add(1);
                    f
                }
                Ok(Err(e)) => return Err(e),
                // Idle: drain the server→client direction.
                Err(_) => ClientFrame::Poll { stream_id },
            };
            match self
                .onion_round_trip(
                    &mut rng,
                    &path,
                    exit_addr,
                    first_peer,
                    &first_addr,
                    &frame.encode(),
                )
                .await?
            {
                ExitFrame::Data(d) => {
                    if !d.is_empty() {
                        self.counters
                            .down
                            .fetch_add(d.len() as u64, std::sync::atomic::Ordering::Relaxed);
                        io.write_all(&d).await?;
                    }
                }
                ExitFrame::Eof => {
                    io.flush().await?;
                    return Ok(());
                }
                ExitFrame::Err(_) => return Ok(()),
            }
        }
    }
}
