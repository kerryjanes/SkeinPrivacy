//! The node daemon engine: own a libp2p swarm, publish the operator-signed
//! [`NodeDescriptor`] into the Kademlia DHT, and (in tests / future M7 socket wiring)
//! resolve peers. The relay/exit cell processing lives in [`weft_net::node::Relay`],
//! exercised end-to-end by the network simulator; binding it to live sockets is the
//! M7 client/OS step.
//!
// `resolve`/`bootstrap`/`peer_id` are the client/test entrypoints + the M7 socket loop;
// the binary only listens + publishes, so they are not all used from `main`.
#![allow(dead_code)]

use futures::StreamExt;
use libp2p::kad;
use libp2p::swarm::SwarmEvent;
use libp2p::{identity, Multiaddr, Swarm};
use weft_net::discovery::{
    build_swarm, descriptor_signing_bytes, record_key, sign_descriptor, validate_record,
    NodeDescriptor, WeftBehaviour, WeftBehaviourEvent,
};
use weft_net::keys::WeftKeypair;
use weft_net::Result;

pub struct Daemon {
    pub swarm: Swarm<WeftBehaviour>,
    kp: WeftKeypair,
    node_id: u64,
    geo: u32,
    capabilities: u32,
    listen: Multiaddr,
}

impl Daemon {
    pub fn new(
        kp: WeftKeypair,
        node_id: u64,
        geo: u32,
        capabilities: u32,
        listen: Multiaddr,
        memory: bool,
    ) -> Result<Self> {
        let swarm = build_swarm(identity::Keypair::generate_ed25519(), memory)?;
        Ok(Self {
            swarm,
            kp,
            node_id,
            geo,
            capabilities,
            listen,
        })
    }

    pub fn listen(&mut self) -> Result<()> {
        self.swarm
            .listen_on(self.listen.clone())
            .map_err(|e| weft_net::NetError::Noise(format!("{e:?}")))?;
        self.swarm
            .behaviour_mut()
            .kad
            .set_mode(Some(kad::Mode::Server));
        Ok(())
    }

    fn descriptor(&self, issued_at: i64) -> NodeDescriptor {
        NodeDescriptor {
            operator: self.kp.operator_pubkey(),
            node_id: self.node_id,
            multiaddr: self.listen.to_string(),
            noise_static_pub: self.kp.static_public(),
            onion_pub: self.kp.onion_public(),
            capabilities: self.capabilities,
            geo: self.geo,
            issued_at,
        }
    }

    /// Publish the operator-signed descriptor into the DHT keyed by
    /// `sha256(operator ‖ node_id)`.
    pub fn publish(&mut self, issued_at: i64) -> Result<()> {
        let desc = self.descriptor(issued_at);
        let sig = self.kp.sign(&descriptor_signing_bytes(&desc));
        let value = sign_descriptor(desc, sig);
        let key = record_key(&self.kp.operator_pubkey(), self.node_id);
        self.swarm
            .behaviour_mut()
            .kad
            .put_record(
                kad::Record {
                    key,
                    value,
                    publisher: None,
                    expires: None,
                },
                kad::Quorum::One,
            )
            .map_err(|e| weft_net::NetError::Noise(format!("{e:?}")))?;
        Ok(())
    }

    pub fn bootstrap(&mut self, peer: &libp2p::PeerId, addr: Multiaddr) {
        self.swarm.behaviour_mut().kad.add_address(peer, addr);
    }

    pub fn peer_id(&self) -> libp2p::PeerId {
        *self.swarm.local_peer_id()
    }

    /// Drive the swarm one event (for embedding in a select loop).
    pub async fn next_event(&mut self) -> SwarmEvent<WeftBehaviourEvent> {
        self.swarm.select_next_some().await
    }

    /// Resolve a node descriptor from the DHT (used by clients / tests).
    pub async fn resolve(&mut self, operator: &[u8; 32], node_id: u64) -> Result<NodeDescriptor> {
        let key = record_key(operator, node_id);
        self.swarm.behaviour_mut().kad.get_record(key.clone());
        loop {
            match self.swarm.select_next_some().await {
                SwarmEvent::ConnectionEstablished { .. } => {
                    self.swarm.behaviour_mut().kad.get_record(key.clone());
                }
                SwarmEvent::Behaviour(WeftBehaviourEvent::Kad(
                    kad::Event::OutboundQueryProgressed {
                        result: kad::QueryResult::GetRecord(res),
                        ..
                    },
                )) => match res {
                    Ok(kad::GetRecordOk::FoundRecord(rec)) => {
                        return validate_record(&record_key(operator, node_id), &rec.record.value);
                    }
                    _ => {
                        self.swarm.behaviour_mut().kad.get_record(key.clone());
                    }
                },
                _ => {}
            }
        }
    }
}
