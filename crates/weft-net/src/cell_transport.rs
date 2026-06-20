//! The `/weft/cell/1.0.0` libp2p request/response protocol that carries onion cells
//! over real connections (M7): a relay receives a [`Cell`] request, peels one layer,
//! forwards downstream (parking its upstream channel) or delivers at the exit, then
//! seals the reverse-path reply back upstream. The request payload is a serde [`Cell`];
//! the response is a [`CellResponse`] so a dropped/tampered cell is distinguishable
//! from a real empty reply. CBOR is the wire codec.

use libp2p::request_response::{self, ProtocolSupport};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};

use crate::sphinx::Cell;

/// The protocol id negotiated on each cell stream.
pub const CELL_PROTOCOL: StreamProtocol = StreamProtocol::new("/weft/cell/1.0.0");

/// Error sentinels carried in a [`CellResponse::Err`] back up the circuit.
pub mod err {
    /// Onion authentication / peel failure (tampered or misrouted cell).
    pub const PEEL: u8 = 1;
    /// Per-client rate limit exceeded.
    pub const RATE_LIMITED: u8 = 2;
    /// Exit refused the destination (content opt-out).
    pub const OPT_OUT: u8 = 3;
    /// A downstream hop failed or timed out.
    pub const DOWNSTREAM: u8 = 4;
    /// The next hop could not be resolved / dialed.
    pub const UNRESOLVABLE: u8 = 5;
}

/// The response to a cell request: either the sealed reverse-path reply bytes, or an
/// error sentinel (see [`err`]).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CellResponse {
    Reply(Vec<u8>),
    Err(u8),
}

/// The cell-transport behaviour: a CBOR request/response over [`Cell`] → [`CellResponse`].
pub type CellBehaviour = request_response::cbor::Behaviour<Cell, CellResponse>;

/// Construct the cell behaviour with a generous request timeout (forward chains
/// compound latency, and a relay may do a mid-circuit DHT lookup to resolve the next hop).
pub fn cell_behaviour() -> CellBehaviour {
    let cfg = request_response::Config::default()
        .with_request_timeout(std::time::Duration::from_secs(30));
    request_response::cbor::Behaviour::new([(CELL_PROTOCOL, ProtocolSupport::Full)], cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sphinx::{create_onion, OnionHop};
    use curve25519_dalek::{constants::RISTRETTO_BASEPOINT_POINT, scalar::Scalar};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn cell_cbor_round_trips() {
        let mut rng = StdRng::seed_from_u64(1);
        let secret = Scalar::random(&mut rng);
        let onion_pub = (RISTRETTO_BASEPOINT_POINT * secret).compress().to_bytes();
        let hop = OnionHop {
            onion_pub,
            addr: [7u8; 32],
            is_exit: true,
        };
        let (cell, _keys) = create_onion(&mut rng, &[hop], b"payload").unwrap();

        let bytes = serde_cbor_roundtrip(&cell);
        assert_eq!(bytes.alpha, cell.alpha);
        assert_eq!(bytes.beta, cell.beta);
        assert_eq!(bytes.gamma, cell.gamma);
        assert_eq!(bytes.delta, cell.delta);
    }

    #[test]
    fn cell_response_cbor_round_trips() {
        for r in [
            CellResponse::Reply(vec![1, 2, 3, 4]),
            CellResponse::Reply(Vec::new()),
            CellResponse::Err(err::PEEL),
            CellResponse::Err(err::DOWNSTREAM),
        ] {
            assert_eq!(serde_cbor_roundtrip(&r), r);
        }
    }

    // libp2p's cbor codec uses ciborium under the hood; mirror it here for the unit test.
    fn serde_cbor_roundtrip<T>(v: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let mut buf = Vec::new();
        ciborium::into_writer(v, &mut buf).unwrap();
        ciborium::from_reader(&buf[..]).unwrap()
    }
}
