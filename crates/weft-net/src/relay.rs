//! The relay/exit event loop that binds [`crate::node::Relay`] to libp2p sockets
//! over the `/weft/cell/1.0.0` protocol. Cells flow recursively: a hop peels one layer,
//! and if it must forward it sends the cell downstream and PARKS its upstream response
//! channel keyed by the outbound request id; when the downstream reply arrives it
//! `reply_seal`s with its hop key and answers upstream. Exits deliver to a destination
//! handler and seal the reply synchronously. Fully event-driven — no inline awaits, so a
//! hop never blocks a task while waiting on its downstream.
//!
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cell_transport::{err, CellResponse};
use crate::discovery::{WeftBehaviour, WeftBehaviourEvent};
use crate::exit::Exit;
use crate::keys::WeftKeypair;
use crate::node::Relay;
use crate::sphinx::{reply_seal, Cell, Peeled};
use futures::StreamExt;
use libp2p::request_response::{self, OutboundRequestId, ResponseChannel};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use tokio::sync::mpsc;

/// How a relay obtains the wall-clock `now` fed to the rate limiter.
#[derive(Clone, Copy)]
pub enum Clock {
    System,
    Fixed(u64),
}

impl Clock {
    pub(crate) fn now(&self) -> u64 {
        match self {
            Clock::System => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            Clock::Fixed(t) => *t,
        }
    }
}

/// A parked upstream channel awaiting a downstream reply for one forwarded cell.
struct Parked {
    upstream: ResponseChannel<CellResponse>,
    reply_key: [u8; 32],
}

/// The exit destination handler (async + concurrent) — see [`crate::exit::Exit`]. Shared so
/// many cells' egress I/O run at once.
pub type ExitHandler = Arc<dyn Exit>;

/// A completed exit task: the reply bytes + the upstream channel + hop key to seal them.
struct ExitDone {
    channel: ResponseChannel<CellResponse>,
    reply_key: [u8; 32],
    reply: Vec<u8>,
}

/// Owns the swarm, the relay engine, the address book, and the in-flight parked channels.
pub struct RelayService {
    swarm: Swarm<WeftBehaviour>,
    relay: Relay,
    clock: Clock,
    addrbook: HashMap<[u8; 32], (PeerId, Multiaddr)>,
    pending: HashMap<OutboundRequestId, Parked>,
    exit: ExitHandler,
    // Exit I/O runs in spawned tasks (so one slow egress never blocks the relay loop) and
    // reports completion here; the event loop seals + answers upstream.
    exit_tx: mpsc::UnboundedSender<ExitDone>,
    exit_rx: mpsc::UnboundedReceiver<ExitDone>,
}

impl RelayService {
    pub fn new(
        swarm: Swarm<WeftBehaviour>,
        relay: Relay,
        clock: Clock,
        exit: ExitHandler,
    ) -> Self {
        let (exit_tx, exit_rx) = mpsc::unbounded_channel();
        Self {
            swarm,
            relay,
            clock,
            addrbook: HashMap::new(),
            pending: HashMap::new(),
            exit,
            exit_tx,
            exit_rx,
        }
    }

    pub fn swarm_mut(&mut self) -> &mut Swarm<WeftBehaviour> {
        &mut self.swarm
    }

    pub fn relay(&self) -> &Relay {
        &self.relay
    }

    pub fn relay_mut(&mut self) -> &mut Relay {
        &mut self.relay
    }

    /// Seed the address book so a peeled `next_addr` (= a DHT record key) resolves to a
    /// dialable peer without a mid-relay DHT lookup. The address is registered with both
    /// Kademlia and the swarm (so request_response can dial it).
    pub fn add_route(&mut self, routing_addr: [u8; 32], peer: PeerId, addr: Multiaddr) {
        self.swarm
            .behaviour_mut()
            .kad
            .add_address(&peer, addr.clone());
        self.swarm.add_peer_address(peer, addr.clone());
        self.addrbook.insert(routing_addr, (peer, addr));
    }

    /// Drive one event: either a swarm event, or a completed exit task whose reply we now
    /// seal and answer upstream. Decoupling the two means a slow egress never blocks the
    /// relay's event loop, so many connections flow concurrently.
    pub async fn step(&mut self) {
        tokio::select! {
            event = self.swarm.select_next_some() => {
                if let SwarmEvent::Behaviour(WeftBehaviourEvent::Cell(ev)) = event {
                    self.handle_cell(ev).await;
                }
            }
            Some(done) = self.exit_rx.recv() => {
                let sealed = reply_seal(&done.reply_key, &done.reply);
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .cell
                    .send_response(done.channel, CellResponse::Reply(sealed));
            }
        }
    }

    async fn handle_cell(&mut self, ev: request_response::Event<Cell, CellResponse>) {
        match ev {
            request_response::Event::Message { peer, message } => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => self.on_request(peer, request, channel).await,
                request_response::Message::Response {
                    request_id,
                    response,
                } => self.on_response(request_id, response),
            },
            request_response::Event::OutboundFailure { request_id, .. } => {
                // a downstream hop failed — tell our upstream the circuit broke.
                if let Some(parked) = self.pending.remove(&request_id) {
                    let _ = self
                        .swarm
                        .behaviour_mut()
                        .cell
                        .send_response(parked.upstream, CellResponse::Err(err::DOWNSTREAM));
                }
            }
            _ => {}
        }
    }

    async fn on_request(
        &mut self,
        peer: PeerId,
        cell: Cell,
        channel: ResponseChannel<CellResponse>,
    ) {
        // The relay meters its immediate upstream (it cannot know the origin without
        // breaking recipient-hiding); identify the upstream by its 32-byte peer id.
        let client = peer_bytes(&peer);
        let now = self.clock.now();
        let respond = |svm: &mut Swarm<WeftBehaviour>, ch, r| {
            let _ = svm.behaviour_mut().cell.send_response(ch, r);
        };

        match self.relay.process(client, &cell, now) {
            Ok(Peeled::Exit {
                dest,
                payload,
                reply_key,
            }) => {
                // Run the exit's real I/O in a task so it never blocks the relay loop; it
                // reports back through `exit_tx` and the event loop answers upstream. This is
                // what lets one exit serve many simultaneous connections.
                let exit = self.exit.clone();
                let tx = self.exit_tx.clone();
                tokio::spawn(async move {
                    let reply = exit.handle(client, dest, &payload).await;
                    let _ = tx.send(ExitDone {
                        channel,
                        reply_key,
                        reply,
                    });
                });
            }
            Ok(Peeled::Forward {
                next_addr,
                cell,
                reply_key,
            }) => match self.resolve(&next_addr) {
                Some(peer) => {
                    let id = self.swarm.behaviour_mut().cell.send_request(&peer, cell);
                    self.pending.insert(
                        id,
                        Parked {
                            upstream: channel,
                            reply_key,
                        },
                    );
                }
                None => respond(
                    &mut self.swarm,
                    channel,
                    CellResponse::Err(err::UNRESOLVABLE),
                ),
            },
            Err(e) => {
                let code = match e {
                    crate::NetError::RateLimited => err::RATE_LIMITED,
                    crate::NetError::ContentOptOut => err::OPT_OUT,
                    _ => err::PEEL,
                };
                respond(&mut self.swarm, channel, CellResponse::Err(code));
            }
        }
    }

    fn on_response(&mut self, request_id: OutboundRequestId, response: CellResponse) {
        let Some(parked) = self.pending.remove(&request_id) else {
            return;
        };
        let out = match response {
            CellResponse::Reply(downstream) => {
                CellResponse::Reply(reply_seal(&parked.reply_key, &downstream))
            }
            err @ CellResponse::Err(_) => err,
        };
        let _ = self
            .swarm
            .behaviour_mut()
            .cell
            .send_response(parked.upstream, out);
    }

    /// Resolve a routing address (DHT record key) to a dialable peer via the address book.
    fn resolve(&self, next_addr: &[u8; 32]) -> Option<PeerId> {
        self.addrbook.get(next_addr).map(|(p, _)| *p)
    }
}

/// A relay's metered upstream client key = the libp2p peer id bytes (multihash → its
/// 32-byte ed25519 digest tail for our deterministic ed25519 peers).
pub(crate) fn peer_bytes(peer: &PeerId) -> [u8; 32] {
    let b = peer.to_bytes();
    let mut out = [0u8; 32];
    let tail = &b[b.len().saturating_sub(32)..];
    out[32 - tail.len()..].copy_from_slice(tail);
    out
}

/// Build a [`Relay`] from a node keypair.
pub fn make_relay(kp: &WeftKeypair, node_id: u64, window_start: u64) -> Relay {
    Relay::new(
        kp.operator_pubkey(),
        node_id,
        kp.onion_secret(),
        window_start,
    )
}

/// Client helper: send a built cell to the first hop and await the sealed reply. Drives a
/// dedicated client swarm. Returns the `CellResponse`.
pub async fn send_cell(
    swarm: &mut Swarm<WeftBehaviour>,
    first_peer: PeerId,
    first_addr: Multiaddr,
    cell: Cell,
) -> CellResponse {
    swarm.add_peer_address(first_peer, first_addr);
    let req_id = swarm.behaviour_mut().cell.send_request(&first_peer, cell);
    loop {
        if let SwarmEvent::Behaviour(WeftBehaviourEvent::Cell(ev)) = swarm.select_next_some().await
        {
            match ev {
                request_response::Event::Message {
                    message:
                        request_response::Message::Response {
                            request_id,
                            response,
                        },
                    ..
                } if request_id == req_id => return response,
                request_response::Event::OutboundFailure { request_id, .. }
                    if request_id == req_id =>
                {
                    return CellResponse::Err(err::DOWNSTREAM);
                }
                _ => {}
            }
        }
    }
}
