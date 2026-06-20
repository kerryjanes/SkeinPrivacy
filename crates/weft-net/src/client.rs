//! Client side of the data plane: turn a selected circuit into an onion cell and open
//! the layered reverse-path reply. The client maps each [`NodeRecord`] hop to an
//! [`OnionHop`] whose `addr` is the NEXT hop's id (the last hop carries the
//! destination), so each relay learns only where to forward next.

use rand::{CryptoRng, RngCore};

use crate::error::Result;
use crate::selection::NodeRecord;
use crate::sphinx::{self, Cell, OnionHop};

/// A built circuit: the ordered path plus the per-hop reply keys.
pub struct Circuit {
    pub path: Vec<NodeRecord>,
    pub reply_keys: Vec<[u8; 32]>,
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
