//! Weft VPN engine.
//!
//! Turns the Weft onion data plane (`weft-net`) into a usable VPN: real application
//! traffic is split into [`stream`] frames, each carried inside one onion cell through a
//! 3–5 hop circuit, and **egresses to the real internet at an exit node**. Two standard
//! front-ends feed the same engine — a local SOCKS5 proxy and a **VLESS gateway** (the
//! protocol V2Box / Happ / any sing-box / Xray client speaks) — and the same [`exit`] runs
//! on operator nodes to dial the real destination. OS-level capture is delegated to those
//! mature clients, so there is no hand-rolled TUN/routing code here.

pub mod client_engine;
pub mod exit;
pub mod localnet;
pub mod manifest;
pub mod socks;
pub mod stream;
pub mod vless;
