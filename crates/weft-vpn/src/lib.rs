//! Weft VPN engine.
//!
//! A usable multi-hop privacy VPN: application traffic is carried through the **Tor network**
//! ([`tor_backend`], via the pure-Rust Arti client) — genuine 3-hop onion routing with Tor's
//! battle-tested flow control, lifecycle, and NAT traversal. Two standard front-ends feed the
//! same backend — a local SOCKS5 proxy and a **VLESS gateway** (the protocol V2Box / Happ /
//! any sing-box / Xray client speaks). OS-level capture is delegated to those mature clients,
//! so there is no hand-rolled TUN/routing code here. Weft's value-add is the polished client
//! plus the on-chain incentive layer that rewards users for running Tor relays.

pub mod socks;
pub mod tor_backend;
pub mod vless;
