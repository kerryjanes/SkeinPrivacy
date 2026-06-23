//! Weft VPN engine.
//!
//! Turns the Weft onion data plane (`weft-net`) into a usable VPN: real application
//! traffic is split into [`stream`] frames, each carried inside one onion cell through a
//! 3–5 hop circuit, and **egresses to the real internet at an exit node**. Two front-ends
//! feed the same engine — a local SOCKS5 proxy (no admin) and a system-wide TUN device —
//! and the same [`exit`] runs on operator nodes to dial the real destination.

pub mod client_engine;
pub mod exit;
pub mod stream;
