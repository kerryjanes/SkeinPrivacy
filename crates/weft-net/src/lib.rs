//! Weft networking data plane (M6). Host-target only (std + tokio + libp2p); not SBF.
//!
//! Two-level wrapping per `SPEC.md`: a **WireGuard Noise link layer** ([`noise`])
//! between adjacent peers carries a **single-pass Sphinx onion** ([`sphinx`]) that
//! hides the final recipient from every intermediate hop. Nodes are discovered via a
//! libp2p Kademlia DHT, selected by geo/reputation, and relayed traffic is metered
//! into the byte-identical M4 dual-signed [`receipt`]s.

pub mod client;
pub mod discovery;
pub mod error;
pub mod keys;
pub mod metering;
pub mod node;
pub mod noise;
pub mod receipt;
pub mod selection;
pub mod sphinx;

pub use error::{NetError, Result};
pub use keys::{verify_sig, WeftKeypair};

/// Crate version smoke.
pub const NET_VERSION: &str = "0.1.0";
