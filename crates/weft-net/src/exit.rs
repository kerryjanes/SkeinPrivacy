//! The exit-destination abstraction. A relay that peels the innermost onion layer hands
//! the decrypted payload to an [`Exit`], whose reply is sealed back through the reverse
//! path. The trait is **async + stateful** so a real exit can hold open sockets and do I/O
//! (see `weft-vpn`'s `InternetExit`); a [`Box<dyn Exit>`] is object-safe via a boxed future.

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
/// Returns the reply bytes to seal back through the circuit.
pub trait Exit: Send {
    fn handle<'a>(
        &'a mut self,
        client: [u8; 32],
        dest: [u8; 32],
        payload: &'a [u8],
    ) -> ExitFuture<'a>;
}

/// A trivial exit that echoes `ack:` + the payload. Used by tests and as the daemon's
/// default before a real-egress exit is configured.
pub struct EchoExit;

impl Exit for EchoExit {
    fn handle<'a>(
        &'a mut self,
        _client: [u8; 32],
        _dest: [u8; 32],
        payload: &'a [u8],
    ) -> ExitFuture<'a> {
        let mut r = b"ack:".to_vec();
        r.extend_from_slice(payload);
        Box::pin(async move { r })
    }
}
