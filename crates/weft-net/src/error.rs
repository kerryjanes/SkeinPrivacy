use thiserror::Error;

/// Errors across the Weft data plane.
#[derive(Debug, Error)]
pub enum NetError {
    #[error("noise handshake/transport error: {0}")]
    Noise(String),
    #[error("onion layer authentication failed at hop {0}")]
    OnionAuth(usize),
    #[error("malformed packet: {0}")]
    Malformed(&'static str),
    #[error("invalid circuit: {0}")]
    Circuit(&'static str),
    #[error("no candidate nodes satisfy the selection constraints")]
    NoCandidates,
    #[error("receipt signature verification failed")]
    BadReceiptSig,
    #[error("rate limit exceeded for circuit")]
    RateLimited,
    #[error("exit refuses this destination (content opt-out)")]
    ContentOptOut,
    #[error("serialization error: {0}")]
    Serde(String),
}

impl From<snow::Error> for NetError {
    fn from(e: snow::Error) -> Self {
        NetError::Noise(format!("{e:?}"))
    }
}

pub type Result<T> = core::result::Result<T, NetError>;
