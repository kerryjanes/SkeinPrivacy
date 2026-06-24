//! Streaming relay/exit over the persistent [`crate::circuit`] transport. A circuit is
//! built once: each hop peels the routing HEADER a single time (the expensive ECDH), caches
//! its `reply_key` + the downstream header, then runs two concurrent copy loops over the
//! persistent substreams — forward (peel one payload layer per cell, meter, forward) and
//! reverse (seal one layer per pushed cell, meter, forward upstream). This is the Tor-style
//! "build the circuit, then stream through it" model; no per-cell circuit rebuild, no per-cell
//! substream, and the exit PUSHES return data (no polling). Sphinx routing/privacy and
//! per-hop metering are identical to the legacy [`crate::relay`] path.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use futures::{AsyncReadExt, StreamExt};
use libp2p::{Multiaddr, PeerId, Swarm};
use libp2p_stream::Control;
use tokio::sync::mpsc;

use crate::circuit::{read_frame, write_frame, CircuitId, Frame, CIRCUIT_PROTOCOL};
use crate::discovery::WeftBehaviour;
use crate::node::Relay;
use crate::relay::{peer_bytes, Clock};
use crate::sphinx::{peel_payload, reply_seal_seq, unframe_payload, Header, PAYLOAD_SIZE};

/// The exit boundary for the streaming path. The relay's exit task feeds decoded FORWARD
/// payloads (a `ClientFrame`-encoded blob) into the returned sender, and the exit PUSHES
/// REVERSE payloads (an `ExitFrame`-encoded blob) back through the returned receiver as the
/// real socket produces bytes. Socket I/O lives in the implementor (weft-vpn) so this crate
/// stays transport-only.
pub trait CircuitExit: Send + Sync {
    fn open(&self, client: [u8; 32]) -> ExitChannels;
}

/// One circuit's exit channels: forward payloads in, reverse payloads out.
pub struct ExitChannels {
    pub forward: mpsc::UnboundedSender<Vec<u8>>,
    pub reverse: mpsc::UnboundedReceiver<Vec<u8>>,
}

/// Shared routing table: a peeled `next_addr` (DHT record key) → dialable peer.
type AddrBook = Arc<RwLock<HashMap<[u8; 32], PeerId>>>;

/// Owns the swarm + the shared relay engine and drives the streaming circuit protocol.
pub struct CircuitRelayService {
    swarm: Swarm<WeftBehaviour>,
    relay: Arc<Mutex<Relay>>,
    clock: Clock,
    addrbook: AddrBook,
    exit: Arc<dyn CircuitExit>,
}

impl CircuitRelayService {
    pub fn new(swarm: Swarm<WeftBehaviour>, relay: Relay, clock: Clock, exit: Arc<dyn CircuitExit>) -> Self {
        Self {
            swarm,
            relay: Arc::new(Mutex::new(relay)),
            clock,
            addrbook: Arc::new(RwLock::new(HashMap::new())),
            exit,
        }
    }

    pub fn swarm_mut(&mut self) -> &mut Swarm<WeftBehaviour> {
        &mut self.swarm
    }

    /// A shared handle to the relay engine (for reading metered totals / receipts from
    /// outside the run loop). Grab this before [`Self::run`] consumes `self`.
    pub fn relay(&self) -> Arc<Mutex<Relay>> {
        self.relay.clone()
    }

    /// Seed the address book so a peeled `next_addr` resolves to a dialable peer.
    pub fn add_route(&mut self, routing_addr: [u8; 32], peer: PeerId, addr: Multiaddr) {
        self.swarm.behaviour_mut().kad.add_address(&peer, addr.clone());
        self.swarm.add_peer_address(peer, addr);
        self.addrbook.write().unwrap().insert(routing_addr, peer);
    }

    /// Run the relay forever: drive the swarm and accept inbound circuit substreams,
    /// spawning a per-circuit task for each.
    pub async fn run(mut self) {
        let mut control = self.swarm.behaviour().circuit.new_control();
        let mut incoming = control
            .accept(CIRCUIT_PROTOCOL)
            .expect("accept circuit protocol");
        loop {
            tokio::select! {
                _ = self.swarm.select_next_some() => {}
                Some((peer, stream)) = incoming.next() => {
                    tokio::spawn(handle_circuit(
                        peer,
                        stream,
                        self.relay.clone(),
                        self.addrbook.clone(),
                        control.clone(),
                        self.exit.clone(),
                        self.clock,
                    ));
                }
            }
        }
    }
}

/// Handle one inbound circuit substream: read OPEN, peel the header once, then either relay
/// (forward + reverse loops) or hand off to the exit.
async fn handle_circuit(
    peer: PeerId,
    upstream: libp2p::Stream,
    relay: Arc<Mutex<Relay>>,
    addrbook: AddrBook,
    control: Control,
    exit: Arc<dyn CircuitExit>,
    clock: Clock,
) {
    let (mut up_r, up_w) = upstream.split();
    let client = peer_bytes(&peer);

    let (circuit_id, header) = match read_frame(&mut up_r).await {
        Ok(Some(Frame::Open { circuit_id, header })) => (circuit_id, header),
        _ => return, // first frame must be OPEN
    };

    let ph = match relay.lock().unwrap().process_header(client, &header, clock.now()) {
        Ok(ph) => ph,
        Err(_) => return,
    };

    if ph.is_exit {
        exit_circuit(client, ph.reply_key, ph.payload_len, up_r, up_w, relay, exit, clock).await;
    } else {
        let Some(fwd) = ph.fwd_header else { return };
        relay_circuit(
            client,
            circuit_id,
            ph.addr,
            fwd,
            ph.reply_key,
            ph.payload_len,
            up_r,
            up_w,
            relay,
            addrbook,
            control,
            clock,
        )
        .await;
    }
}

/// Relay hop: open the downstream substream, then run forward + reverse copy loops.
#[allow(clippy::too_many_arguments)]
async fn relay_circuit(
    client: [u8; 32],
    circuit_id: CircuitId,
    next_addr: [u8; 32],
    fwd_header: Header,
    reply_key: [u8; 32],
    payload_len: usize,
    mut up_r: futures::io::ReadHalf<libp2p::Stream>,
    mut up_w: futures::io::WriteHalf<libp2p::Stream>,
    relay: Arc<Mutex<Relay>>,
    addrbook: AddrBook,
    mut control: Control,
    clock: Clock,
) {
    // The peeled header gives this hop the next hop's routing address; resolve it to a peer.
    let next_peer = match addrbook.read().unwrap().get(&next_addr).copied() {
        Some(p) => p,
        None => return, // unresolvable next hop
    };

    let down = match control.open_stream(next_peer, CIRCUIT_PROTOCOL).await {
        Ok(s) => s,
        Err(_) => return,
    };
    let (mut down_r, mut down_w) = down.split();
    if write_frame(&mut down_w, &Frame::Open { circuit_id, header: fwd_header })
        .await
        .is_err()
    {
        return;
    }

    // Forward: upstream FWD -> peel one layer -> meter -> downstream FWD.
    let r1 = relay.clone();
    let forward = async move {
        while let Ok(Some(frame)) = read_frame(&mut up_r).await {
            match frame {
                Frame::Fwd { seq, delta } => {
                    let wire = 8 + delta.len() as u64;
                    if !r1.lock().unwrap().admit(client, wire, clock.now()) {
                        continue; // rate-limited: drop, never billed
                    }
                    let Ok(inner) = peel_payload(&reply_key, seq, payload_len, &delta) else {
                        continue; // bad cell: drop, never billed
                    };
                    r1.lock().unwrap().record(client, wire);
                    let mut d = inner;
                    d.resize(PAYLOAD_SIZE, 0);
                    if write_frame(&mut down_w, &Frame::Fwd { seq, delta: d }).await.is_err() {
                        break;
                    }
                }
                Frame::Close => {
                    let _ = write_frame(&mut down_w, &Frame::Close).await;
                    break;
                }
                _ => {}
            }
        }
    };

    // Reverse: downstream REV -> seal one layer -> meter -> upstream REV.
    let r2 = relay.clone();
    let reverse = async move {
        while let Ok(Some(frame)) = read_frame(&mut down_r).await {
            match frame {
                Frame::Rev { seq, sealed } => {
                    let wire = 8 + sealed.len() as u64;
                    if r2.lock().unwrap().admit(client, wire, clock.now()) {
                        r2.lock().unwrap().record(client, wire);
                    }
                    let sealed2 = reply_seal_seq(&reply_key, seq, &sealed);
                    if write_frame(&mut up_w, &Frame::Rev { seq, sealed: sealed2 }).await.is_err() {
                        break;
                    }
                }
                Frame::Close => break,
                _ => {}
            }
        }
    };

    tokio::join!(forward, reverse);
}

/// Exit hop: decode forward cells into `ClientFrame` payloads for the exit, and seal the
/// exit's pushed reverse payloads back upstream with an exit-owned reverse `seq`.
#[allow(clippy::too_many_arguments)]
async fn exit_circuit(
    client: [u8; 32],
    reply_key: [u8; 32],
    payload_len: usize,
    mut up_r: futures::io::ReadHalf<libp2p::Stream>,
    mut up_w: futures::io::WriteHalf<libp2p::Stream>,
    relay: Arc<Mutex<Relay>>,
    exit: Arc<dyn CircuitExit>,
    clock: Clock,
) {
    let chans = exit.open(client);
    let forward_sink = chans.forward;
    let mut reverse = chans.reverse;

    // Forward: upstream FWD -> peel one layer -> ClientFrame payload -> exit.
    let r1 = relay.clone();
    let forward = async move {
        while let Ok(Some(frame)) = read_frame(&mut up_r).await {
            match frame {
                Frame::Fwd { seq, delta } => {
                    let wire = 8 + delta.len() as u64;
                    if !r1.lock().unwrap().admit(client, wire, clock.now()) {
                        continue;
                    }
                    let Ok(inner) = peel_payload(&reply_key, seq, payload_len, &delta) else {
                        continue;
                    };
                    // The exit's payload is length-framed inside the fixed-size cell; strip the
                    // padding to recover the exact `ClientFrame` bytes.
                    let Ok(payload) = unframe_payload(&inner) else {
                        continue;
                    };
                    r1.lock().unwrap().record(client, wire);
                    if forward_sink.send(payload).is_err() {
                        break;
                    }
                }
                Frame::Close => break,
                _ => {}
            }
        }
    };

    // Reverse: exit pushes payloads -> seal with this exit's reverse seq -> upstream REV.
    let reverse_loop = async move {
        let mut seq: u64 = 0;
        while let Some(payload) = reverse.recv().await {
            let sealed = reply_seal_seq(&reply_key, seq, &payload);
            seq = seq.wrapping_add(1);
            if write_frame(&mut up_w, &Frame::Rev { seq: seq - 1, sealed }).await.is_err() {
                break;
            }
        }
    };

    tokio::join!(forward, reverse_loop);
}
