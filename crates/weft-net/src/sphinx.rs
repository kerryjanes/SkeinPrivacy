//! Single-pass **Sphinx** onion (Danezis–Goldberg) over the Ristretto prime-order
//! group. The client derives every per-hop key locally from one ephemeral scalar via
//! the blinding chain, wraps the payload in nested layers, and sends one fixed-size
//! cell. Each hop recomputes its own shared secret, verifies the header MAC, learns
//! ONLY its next hop, peels exactly one layer, and forwards — so no intermediate hop
//! sees the final recipient or the payload. This is the spec's "outer envelope that
//! hides the recipient from each intermediate node"; it rides inside the WireGuard
//! ([`crate::noise`]) link tunnel between adjacent peers.

use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use chacha20poly1305::aead::{Aead, KeyInit as AeadKeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use curve25519_dalek::{
    constants::RISTRETTO_BASEPOINT_POINT, ristretto::CompressedRistretto, scalar::Scalar,
};
use hkdf::Hkdf;
use poly1305::Poly1305;
use rand::{CryptoRng, RngCore};
use sha2::Sha256;

use crate::error::{NetError, Result};

// ---- parameters ----
pub const MAX_HOPS: usize = 5;
/// flags(1) ‖ payload_len(2 LE) ‖ addr(32) ‖ next_mac(16)
pub const ADDR_SIZE: usize = 32;
pub const MAC_SIZE: usize = 16;
pub const HOP_SIZE: usize = 1 + 2 + ADDR_SIZE + MAC_SIZE; // 51
pub const BETA_LEN: usize = MAX_HOPS * HOP_SIZE; // 255
const STREAM_LEN: usize = BETA_LEN + HOP_SIZE; // 306
/// Framed inner plaintext length: len(2) ‖ data ‖ pad.
pub const INNER_LEN: usize = 1024;
pub const MAX_PAYLOAD: usize = INNER_LEN - 2;
/// Each AEAD layer adds a 16-byte tag; the wire payload is padded to the max.
pub const PAYLOAD_SIZE: usize = INNER_LEN + MAX_HOPS * 16; // 1104

const FLAG_EXIT: u8 = 0x01;

/// One hop on the forward path (the client builds the onion from these).
#[derive(Clone)]
pub struct OnionHop {
    /// Compressed Ristretto onion public key of the hop.
    pub onion_pub: [u8; 32],
    /// What this hop forwards to: the next hop's id, or (exit) the destination id.
    pub addr: [u8; 32],
    pub is_exit: bool,
}

/// The fixed-size wire cell. `delta` is always [`PAYLOAD_SIZE`] regardless of hop
/// count, so a 3-hop and a 5-hop circuit are indistinguishable by length.
#[derive(Clone)]
pub struct Cell {
    pub alpha: [u8; 32],
    pub beta: Vec<u8>, // BETA_LEN
    pub gamma: [u8; MAC_SIZE],
    pub delta: Vec<u8>, // PAYLOAD_SIZE
}

/// What a hop should do after peeling one layer. `reply_key` lets the hop seal one
/// layer of a reverse-path reply (a VPN is bidirectional); the client, holding every
/// hop's reply key, opens the layered reply.
pub enum Peeled {
    Forward {
        next_addr: [u8; 32],
        cell: Cell,
        reply_key: [u8; 32],
    },
    Exit {
        dest: [u8; 32],
        payload: Vec<u8>,
        reply_key: [u8; 32],
    },
}

// ---- crypto helpers ----
fn hkdf32(secret: &[u8; 32], label: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, secret);
    let mut out = [0u8; 32];
    hk.expand(label, &mut out).expect("32 < 255*32");
    out
}
fn rho_stream(secret: &[u8; 32], len: usize) -> Vec<u8> {
    let key = hkdf32(secret, b"weft-sphinx-rho");
    let mut buf = vec![0u8; len];
    ChaCha20::new((&key).into(), (&[0u8; 12]).into()).apply_keystream(&mut buf);
    buf
}
fn header_mac(secret: &[u8; 32], beta: &[u8]) -> [u8; 16] {
    let key = hkdf32(secret, b"weft-sphinx-mu");
    let mac = Poly1305::new((&key).into()).compute_unpadded(beta);
    mac.into()
}
fn payload_key(secret: &[u8; 32]) -> [u8; 32] {
    hkdf32(secret, b"weft-sphinx-pi")
}
fn blinding_factor(alpha: &[u8; 32], secret: &[u8; 32]) -> Scalar {
    let mut h = [0u8; 64];
    let hk = Hkdf::<Sha256>::new(
        Some(b"weft-sphinx-blind"),
        &[alpha.as_slice(), secret].concat(),
    );
    hk.expand(b"b", &mut h).expect("64 < 255*32");
    Scalar::from_bytes_mod_order_wide(&h)
}

fn aead_seal(key: &[u8; 32], pt: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(key.into())
        .encrypt(&Nonce::default(), pt)
        .expect("aead seal")
}
fn aead_open(key: &[u8; 32], ct: &[u8]) -> Result<Vec<u8>> {
    ChaCha20Poly1305::new(key.into())
        .decrypt(&Nonce::default(), ct)
        .map_err(|_| NetError::Malformed("payload aead"))
}

fn frame_payload(data: &[u8]) -> Result<[u8; INNER_LEN]> {
    if data.len() > MAX_PAYLOAD {
        return Err(NetError::Malformed("payload too large"));
    }
    let mut out = [0u8; INNER_LEN];
    out[0..2].copy_from_slice(&(data.len() as u16).to_le_bytes());
    out[2..2 + data.len()].copy_from_slice(data);
    Ok(out)
}
fn unframe_payload(inner: &[u8]) -> Result<Vec<u8>> {
    if inner.len() < 2 {
        return Err(NetError::Malformed("inner too short"));
    }
    let len = u16::from_le_bytes([inner[0], inner[1]]) as usize;
    if 2 + len > inner.len() {
        return Err(NetError::Malformed("inner length"));
    }
    Ok(inner[2..2 + len].to_vec())
}

// ---- shared-secret derivation along the path ----
fn path_secrets(ephemeral: &Scalar, hops: &[OnionHop]) -> Result<(Vec<[u8; 32]>, [u8; 32])> {
    let mut secrets = Vec::with_capacity(hops.len());
    let mut priv_acc = *ephemeral;
    let alpha0 = (RISTRETTO_BASEPOINT_POINT * ephemeral)
        .compress()
        .to_bytes();
    for hop in hops {
        let y = CompressedRistretto(hop.onion_pub)
            .decompress()
            .ok_or(NetError::Malformed("onion pubkey"))?;
        let alpha_i = (RISTRETTO_BASEPOINT_POINT * priv_acc).compress().to_bytes();
        let s_i = (y * priv_acc).compress().to_bytes();
        secrets.push(s_i);
        let b_i = blinding_factor(&alpha_i, &s_i);
        priv_acc *= b_i;
    }
    Ok((secrets, alpha0))
}

fn build_block(hop: &OnionHop, payload_len: u16, next_mac: [u8; 16]) -> [u8; HOP_SIZE] {
    let mut b = [0u8; HOP_SIZE];
    b[0] = if hop.is_exit { FLAG_EXIT } else { 0 };
    b[1..3].copy_from_slice(&payload_len.to_le_bytes());
    b[3..3 + ADDR_SIZE].copy_from_slice(&hop.addr);
    b[3 + ADDR_SIZE..].copy_from_slice(&next_mac);
    b
}

fn filler(secrets: &[[u8; 32]]) -> Vec<u8> {
    let n = secrets.len();
    let mut phi: Vec<u8> = Vec::new();
    for s in secrets.iter().take(n.saturating_sub(1)) {
        let stream = rho_stream(s, STREAM_LEN);
        let i = phi.len() / HOP_SIZE;
        phi.resize(phi.len() + HOP_SIZE, 0);
        let start = BETA_LEN - i * HOP_SIZE;
        for (k, p) in phi.iter_mut().enumerate() {
            *p ^= stream[start + k];
        }
    }
    phi
}

/// Build the onion cell for `payload` routed along `hops` (3–5 hops, last = exit).
/// Returns the cell plus the per-hop reply keys (in path order) the client keeps to
/// open a layered reverse-path reply.
pub fn create_onion<R: RngCore + CryptoRng>(
    rng: &mut R,
    hops: &[OnionHop],
    payload: &[u8],
) -> Result<(Cell, Vec<[u8; 32]>)> {
    let n = hops.len();
    if !(1..=MAX_HOPS).contains(&n) {
        return Err(NetError::Circuit("hop count out of range"));
    }
    let ephemeral = Scalar::random(rng);
    let (secrets, alpha0) = path_secrets(&ephemeral, hops)?;

    // payload: nested AEAD, outermost = hop 0.
    let mut cur = frame_payload(payload)?.to_vec();
    for s in secrets.iter().rev() {
        cur = aead_seal(&payload_key(s), &cur);
    }
    let mut delta = cur;
    delta.resize(PAYLOAD_SIZE, 0);

    // header: build beta/gamma backward, with the Sphinx filler at the last hop.
    let fill = filler(&secrets);
    let pad_key = hkdf32(&alpha0, b"weft-sphinx-pad");
    let mut beta = rho_stream(&pad_key, BETA_LEN); // pseudo-random initial pad
    let mut next_mac = [0u8; MAC_SIZE];
    for i in (0..n).rev() {
        let payload_len = (INNER_LEN + (n - i) * 16) as u16;
        let block = build_block(&hops[i], payload_len, next_mac);
        let mut nb = Vec::with_capacity(BETA_LEN);
        nb.extend_from_slice(&block);
        nb.extend_from_slice(&beta[..BETA_LEN - HOP_SIZE]);
        beta = nb;
        let stream = rho_stream(&secrets[i], BETA_LEN);
        for (k, b) in beta.iter_mut().enumerate() {
            *b ^= stream[k];
        }
        if i == n - 1 {
            let off = BETA_LEN - fill.len();
            beta[off..].copy_from_slice(&fill);
        }
        next_mac = header_mac(&secrets[i], &beta);
    }

    let reply_keys = secrets.iter().map(payload_key).collect();
    Ok((
        Cell {
            alpha: alpha0,
            beta,
            gamma: next_mac,
            delta,
        },
        reply_keys,
    ))
}

/// Peel one layer at a hop holding onion secret `y`.
pub fn peel(cell: &Cell, y: &Scalar) -> Result<Peeled> {
    let alpha = CompressedRistretto(cell.alpha)
        .decompress()
        .ok_or(NetError::Malformed("alpha"))?;
    let secret = (alpha * y).compress().to_bytes();

    // header integrity
    if header_mac(&secret, &cell.beta) != cell.gamma {
        return Err(NetError::OnionAuth(0));
    }

    // decrypt routing block + shift
    let mut padded = cell.beta.clone();
    padded.resize(STREAM_LEN, 0);
    let stream = rho_stream(&secret, STREAM_LEN);
    for (k, b) in padded.iter_mut().enumerate() {
        *b ^= stream[k];
    }
    let block = &padded[..HOP_SIZE];
    let beta_next = padded[HOP_SIZE..].to_vec(); // BETA_LEN
    let flags = block[0];
    let payload_len = u16::from_le_bytes([block[1], block[2]]) as usize;
    let mut addr = [0u8; ADDR_SIZE];
    addr.copy_from_slice(&block[3..3 + ADDR_SIZE]);
    let mut next_mac = [0u8; MAC_SIZE];
    next_mac.copy_from_slice(&block[3 + ADDR_SIZE..]);

    // peel one payload layer
    if payload_len > cell.delta.len() {
        return Err(NetError::Malformed("payload_len"));
    }
    let reply_key = payload_key(&secret);
    let inner = aead_open(&reply_key, &cell.delta[..payload_len])?;

    if flags & FLAG_EXIT != 0 {
        let payload = unframe_payload(&inner)?;
        Ok(Peeled::Exit {
            dest: addr,
            payload,
            reply_key,
        })
    } else {
        let b = blinding_factor(&cell.alpha, &secret);
        let alpha_out = (alpha * b).compress().to_bytes();
        let mut delta = inner;
        delta.resize(PAYLOAD_SIZE, 0);
        Ok(Peeled::Forward {
            next_addr: addr,
            cell: Cell {
                alpha: alpha_out,
                beta: beta_next,
                gamma: next_mac,
                delta,
            },
            reply_key,
        })
    }
}

/// Seal one reverse-path reply layer with a hop's `reply_key` (exit seals the
/// innermost layer; each hop toward the client adds an outer layer).
pub fn reply_seal(reply_key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    aead_seal(reply_key, data)
}

/// Open a fully-layered reply at the client. `reply_keys` are in forward path order
/// (entry first); the entry hop's layer is outermost, so we peel in path order.
pub fn reply_open(reply_keys: &[[u8; 32]], sealed: &[u8]) -> Result<Vec<u8>> {
    let mut cur = sealed.to_vec();
    for k in reply_keys {
        cur = aead_open(k, &cur)?;
    }
    Ok(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    struct Node {
        secret: Scalar,
        onion_pub: [u8; 32],
        addr: [u8; 32],
    }
    fn node(rng: &mut StdRng, tag: u8) -> Node {
        let secret = Scalar::random(rng);
        let onion_pub = (RISTRETTO_BASEPOINT_POINT * secret).compress().to_bytes();
        Node {
            secret,
            onion_pub,
            addr: [tag; 32],
        }
    }

    fn route(nodes: &[Node]) -> Vec<OnionHop> {
        nodes
            .iter()
            .enumerate()
            .map(|(i, _)| OnionHop {
                onion_pub: nodes[i].onion_pub,
                // each hop's addr = the NEXT node's addr; last carries the destination.
                addr: if i + 1 < nodes.len() {
                    nodes[i + 1].addr
                } else {
                    [0xde; 32] // destination id
                },
                is_exit: i + 1 == nodes.len(),
            })
            .collect()
    }

    fn run_circuit(hops: usize) {
        let mut rng = StdRng::seed_from_u64(hops as u64);
        let nodes: Vec<Node> = (0..hops).map(|i| node(&mut rng, i as u8)).collect();
        let path = route(&nodes);
        let msg = b"the quick brown fox jumps over the lazy relay";
        let (cell, client_reply_keys) = create_onion(&mut rng, &path, msg).unwrap();
        // fixed cell size regardless of hop count
        assert_eq!(cell.beta.len(), BETA_LEN);
        assert_eq!(cell.delta.len(), PAYLOAD_SIZE);

        let mut current = cell;
        let mut seen_dest = false;
        // each hop's reply key, collected in path order
        let mut hop_reply_keys = Vec::new();
        for (i, n) in nodes.iter().enumerate() {
            match peel(&current, &n.secret).unwrap() {
                Peeled::Forward {
                    next_addr,
                    cell,
                    reply_key,
                } => {
                    assert!(i + 1 < nodes.len());
                    assert_eq!(next_addr, nodes[i + 1].addr); // learns only next hop
                    hop_reply_keys.push(reply_key);
                    current = cell;
                }
                Peeled::Exit {
                    dest,
                    payload,
                    reply_key,
                } => {
                    assert_eq!(i + 1, nodes.len());
                    assert_eq!(&dest, &[0xde; 32]);
                    assert_eq!(payload, msg);
                    hop_reply_keys.push(reply_key);
                    seen_dest = true;
                }
            }
        }
        assert!(seen_dest, "exit must deliver");
        // the keys the hops derived match what the client kept
        assert_eq!(hop_reply_keys, client_reply_keys);

        // reverse path: exit seals innermost, each hop toward the client adds a layer.
        let reply = b"pong from the destination";
        let mut sealed = reply.to_vec();
        for k in hop_reply_keys.iter().rev() {
            sealed = reply_seal(k, &sealed);
        }
        let opened = reply_open(&client_reply_keys, &sealed).unwrap();
        assert_eq!(opened, reply);
    }

    #[test]
    fn onion_roundtrips_3_4_5_hops() {
        for k in [3usize, 4, 5] {
            run_circuit(k);
        }
    }

    #[test]
    fn two_hop_minimum_roundtrips() {
        run_circuit(2);
    }

    #[test]
    fn tampered_header_fails_at_hop() {
        let mut rng = StdRng::seed_from_u64(99);
        let nodes: Vec<Node> = (0..3).map(|i| node(&mut rng, i as u8)).collect();
        let path = route(&nodes);
        let (mut cell, _keys) = create_onion(&mut rng, &path, b"hi").unwrap();
        cell.gamma[0] ^= 1; // corrupt the header MAC
        assert!(matches!(
            peel(&cell, &nodes[0].secret),
            Err(NetError::OnionAuth(_))
        ));
    }

    #[test]
    fn tampered_payload_fails() {
        let mut rng = StdRng::seed_from_u64(7);
        let nodes: Vec<Node> = (0..3).map(|i| node(&mut rng, i as u8)).collect();
        let path = route(&nodes);
        let (mut cell, _keys) = create_onion(&mut rng, &path, b"hi").unwrap();
        cell.delta[0] ^= 1; // corrupt the payload ciphertext
        assert!(peel(&cell, &nodes[0].secret).is_err());
    }
}
