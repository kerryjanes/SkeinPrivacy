//! The exit-destination abstraction. A relay that peels the innermost onion layer hands
//! the decrypted payload to an [`Exit`], whose reply is sealed back through the reverse
//! path. The trait is **async + concurrent**: `handle` takes `&self` (interior mutability)
//! so the relay can run many exits' I/O at once without blocking its event loop — essential
//! for a VPN exit serving many simultaneous connections. A real exit (see `weft-vpn`'s
//! `InternetExit`) holds open sockets behind its own synchronisation.

use std::future::Future;
use std::pin::Pin;

/// The future returned by [`Exit::handle`].
pub type ExitFuture<'a> = Pin<Box<dyn Future<Output = Vec<u8>> + Send + 'a>>;

/// Handles payloads delivered to an exit node.
///
/// - `client`: the metered immediate-upstream peer id bytes (the exit can't see the true
///   circuit origin without breaking recipient-hiding), used to key per-client state.
/// - `dest`: the 32-byte onion destination selector for this exit.
/// - `payload`: the decrypted application bytes (e.g. a `weft-vpn` stream frame).
///
/// Returns the reply bytes to seal back through the circuit. `&self` + `Send + Sync` lets
/// the relay clone an `Arc<dyn Exit>` and spawn each call concurrently.
pub trait Exit: Send + Sync {
    fn handle<'a>(&'a self, client: [u8; 32], dest: [u8; 32], payload: &'a [u8]) -> ExitFuture<'a>;
}

/// A trivial exit that echoes `ack:` + the payload. Used by tests and as the daemon's
/// default before a real-egress exit is configured.
pub struct EchoExit;

impl Exit for EchoExit {
    fn handle<'a>(
        &'a self,
        _client: [u8; 32],
        _dest: [u8; 32],
        payload: &'a [u8],
    ) -> ExitFuture<'a> {
        let mut r = b"ack:".to_vec();
        r.extend_from_slice(payload);
        Box::pin(async move { r })
    }
}
