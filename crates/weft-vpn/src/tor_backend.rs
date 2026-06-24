//! The Weft data plane: route every flow through the **Tor network** via Arti (the pure-Rust
//! Tor client). A multi-hop onion circuit, end-to-end flow control, NAT traversal, exit
//! selection, and 20 years of hardening come for free — Weft keeps the front-ends (SOCKS /
//! VLESS gateway) and the on-chain incentive layer; Tor carries the bytes. This replaces the
//! hand-rolled onion data plane.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use arti_client::{TorClient, TorClientConfig};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tor_rtcompat::PreferredRuntime;

/// Live throughput counters (bytes tunnelled), shared with a UI. Same shape both front-ends
/// already consume.
#[derive(Clone, Default)]
pub struct Counters {
    pub up: Arc<AtomicU64>,
    pub down: Arc<AtomicU64>,
}

/// The Tor-backed data plane. One bootstrapped Tor client serves every flow; each `tunnel`
/// opens a stream over a Tor circuit (Tor builds, reuses, and rotates circuits itself).
pub struct TorBackend {
    tor: Arc<TorClient<PreferredRuntime>>,
    counters: Counters,
}

impl TorBackend {
    /// Bootstrap the Tor client. On first run this fetches the Tor directory consensus
    /// (~10–30 s); afterwards it is cached. Reaching the Tor network requires outbound
    /// internet (works behind NAT — the client never needs to be reachable).
    pub async fn bootstrap() -> io::Result<Self> {
        let tor = TorClient::create_bootstrapped(TorClientConfig::default())
            .await
            .map_err(|e| io::Error::other(format!("tor bootstrap: {e}")))?;
        Ok(Self {
            tor,
            counters: Counters::default(),
        })
    }

    /// Live byte counters (cheap clone; shares the same atomics).
    pub fn counters(&self) -> Counters {
        self.counters.clone()
    }

    /// Tunnel a duplex byte stream `io` (an accepted SOCKS/VLESS flow) to `dst` through Tor.
    /// `udp` is unsupported — Tor carries TCP streams only (browsers fall back to TCP when UDP
    /// egress fails). Same signature as the old engine, so the front-ends are unchanged.
    pub async fn tunnel<S>(&self, dst: SocketAddr, _seed: u64, io: S, udp: bool) -> io::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        if udp {
            return Err(io::Error::other("UDP egress is not supported over Tor (TCP only)"));
        }
        // Connect through Tor to the destination. The front-end already resolved the address;
        // we hand Tor the IP:port and it builds a fresh onion circuit to it.
        let host = dst.ip().to_string();
        let stream = self
            .tor
            .connect((host.as_str(), dst.port()))
            .await
            .map_err(|e| io::Error::other(format!("tor connect {dst}: {e}")))?;
        let mut tor_stream = stream.compat();
        let mut io = io;
        // Full-duplex copy with proper half-close handling (one direction ending shuts down
        // that write side and lets the other finish) — far more robust than a hand-rolled pump.
        let (up, down) = tokio::io::copy_bidirectional(&mut io, &mut tor_stream).await?;
        self.counters.up.fetch_add(up, Ordering::Relaxed);
        self.counters.down.fetch_add(down, Ordering::Relaxed);
        Ok(())
    }
}
