//! Client side of the data plane: turn a selected circuit into an onion cell and open
//! the layered reverse-path reply. The client maps each [`NodeRecord`] hop to an
//! [`OnionHop`] whose `addr` is the NEXT hop's id (the last hop carries the
//! destination), so each relay learns only where to forward next.

use rand::{CryptoRng, RngCore};

use crate::error::Result;
use crate::selection::NodeRecord;
use crate::sphinx::{self, Cell, Header, OnionHop};

/// A built circuit: the ordered path plus the per-hop reply keys.
pub struct Circuit {
    pub path: Vec<NodeRecord>,
    pub reply_keys: Vec<[u8; 32]>,
}

/// A built **streaming** circuit (the persistent-substream data plane): one Sphinx routing
/// header — reused for every cell, so the per-hop ECDH runs once — plus the per-hop keys
/// (forward seal key == reverse open key). The client seals each forward cell at a unique
/// forward `seq` and opens each pushed reverse cell at the exit's reverse `seq`.
pub struct StreamCircuit {
    pub path: Vec<NodeRecord>,
    pub header: Header,
    pub keys: Vec<[u8; 32]>,
}

/// Build a streaming circuit over `path` (exit dials `dest`). The header is built once; cells
/// are sealed cheaply with [`StreamCircuit::seal`].
pub fn build_stream_circuit<R: RngCore + CryptoRng>(
    rng: &mut R,
    path: &[NodeRecord],
    dest: [u8; 32],
) -> Result<StreamCircuit> {
    let hops = onion_hops(path, dest);
    let (header, keys) = sphinx::build_header(rng, &hops)?;
    Ok(StreamCircuit {
        path: path.to_vec(),
        header,
        keys,
    })
}

impl StreamCircuit {
    /// Seal one forward data cell at forward `seq` (must strictly increase per circuit).
    pub fn seal(&self, seq: u64, data: &[u8]) -> Result<Vec<u8>> {
        sphinx::seal_forward_payload(&self.keys, seq, data)
    }

    /// Open one pushed reverse data cell at the exit's reverse `seq`.
    pub fn open(&self, seq: u64, sealed: &[u8]) -> Result<Vec<u8>> {
        sphinx::reply_open_seq(&self.keys, seq, sealed)
    }
}

/// Map a selected path to onion hops, threading `dest` into the exit hop's address.
pub fn onion_hops(path: &[NodeRecord], dest: [u8; 32]) -> Vec<OnionHop> {
    path.iter()
        .enumerate()
        .map(|(i, n)| OnionHop {
            onion_pub: n.onion_pub,
            addr: if i + 1 < path.len() {
                path[i + 1].addr
            } else {
                dest
            },
            is_exit: i + 1 == path.len(),
        })
        .collect()
}

/// Build the onion cell for `payload` over `path`, returning the cell + circuit state.
pub fn build_circuit<R: RngCore + CryptoRng>(
    rng: &mut R,
    path: &[NodeRecord],
    dest: [u8; 32],
    payload: &[u8],
) -> Result<(Cell, Circuit)> {
    let hops = onion_hops(path, dest);
    let (cell, reply_keys) = sphinx::create_onion(rng, &hops, payload)?;
    Ok((
        cell,
        Circuit {
            path: path.to_vec(),
            reply_keys,
        },
    ))
}

/// Open a fully-layered reply from the destination.
pub fn open_reply(circuit: &Circuit, sealed: &[u8]) -> Result<Vec<u8>> {
    sphinx::reply_open(&circuit.reply_keys, sealed)
}
