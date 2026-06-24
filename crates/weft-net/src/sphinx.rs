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
use serde::{Deserialize, Serialize};
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cell {
    pub alpha: [u8; 32],
    pub beta: Vec<u8>, // BETA_LEN
    pub gamma: [u8; MAC_SIZE],
    pub delta: Vec<u8>, // PAYLOAD_SIZE
}

impl Cell {
    /// Total wire byte length (fixed across hop counts) — what a relay meters per cell.
    pub fn wire_len(&self) -> usize {
        32 + self.beta.len() + MAC_SIZE + self.delta.len()
    }
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

/// The Sphinx routing header — `alpha`/`beta`/`gamma` without any payload. It is built
/// once per circuit (the expensive ECDH blinding chain) and reused for every cell, since
/// it peels identically each time (the per-hop secret depends only on `alpha` and the
/// hop's key). A wire data cell is this header (sent once) plus a per-cell `(seq, delta)`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    pub alpha: [u8; 32],
    pub beta: Vec<u8>, // BETA_LEN
    pub gamma: [u8; MAC_SIZE],
}

/// Result of peeling a header ONCE at a hop: the routing decision + this hop's reusable
/// `reply_key` (used to open every forward payload cell and seal every reverse cell) and
/// the `payload_len` it must open. `fwd_header` is the header to forward downstream (the
/// same for every cell on this circuit); `None` means this hop is the exit.
pub struct PeeledHeader {
    pub addr: [u8; 32], // next hop id, or (exit) destination id
    pub is_exit: bool,
    pub reply_key: [u8; 32],
    pub payload_len: usize,
    pub fwd_header: Option<Header>,
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

// AEAD nonce discipline (the crux of persistent circuits): the forward and reverse
// directions reuse the SAME per-hop key (`payload_key` == `reply_key`), and a persistent
// circuit reuses one header — hence one key — across many cells. So every cell MUST get a
// unique 96-bit nonce or ChaCha20-Poly1305 catastrophically reuses a keystream. The nonce
// is `dir(1) ‖ 0(3) ‖ seq(8 LE)`: the direction byte keeps forward/reverse disjoint (they
// share a key), and `seq` is a strictly-monotonic per-circuit, per-direction counter.
const DIR_FWD: u8 = 0;
const DIR_REV: u8 = 1;

fn cell_nonce(dir: u8, seq: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[0] = dir;
    n[4..12].copy_from_slice(&seq.to_le_bytes());
    n
}

fn aead_seal(key: &[u8; 32], nonce: &[u8; 12], pt: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(key.into())
        .encrypt(Nonce::from_slice(nonce), pt)
        .expect("aead seal")
}
fn aead_open(key: &[u8; 32], nonce: &[u8; 12], ct: &[u8]) -> Result<Vec<u8>> {
    ChaCha20Poly1305::new(key.into())
        .decrypt(Nonce::from_slice(nonce), ct)
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

/// Build the Sphinx routing header ONCE for a circuit along `hops` (1–5 hops, last =
/// exit). This runs the expensive part — the ephemeral scalar, the per-hop ECDH blinding
/// chain (`path_secrets`), and the beta/gamma construction — exactly once; the returned
/// `Header` is then reused for every data cell. Also returns the per-hop keys (path order)
/// the client keeps: each doubles as the forward payload key and the reverse reply key.
pub fn build_header<R: RngCore + CryptoRng>(
    rng: &mut R,
    hops: &[OnionHop],
) -> Result<(Header, Vec<[u8; 32]>)> {
    let n = hops.len();
    if !(1..=MAX_HOPS).contains(&n) {
        return Err(NetError::Circuit("hop count out of range"));
    }
    let ephemeral = Scalar::random(rng);
    let (secrets, alpha0) = path_secrets(&ephemeral, hops)?;

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

    let keys = secrets.iter().map(payload_key).collect();
    Ok((
        Header {
            alpha: alpha0,
            beta,
            gamma: next_mac,
        },
        keys,
    ))
}

/// Seal ONE forward data cell's `delta` using the circuit's per-hop `keys` and a unique
/// `seq`. Cheap — N nested AEAD seals (outermost = hop 0), no curve ops. The caller MUST
/// pass a strictly-increasing `seq` per circuit (forward direction) so no nonce repeats.
pub fn seal_forward_payload(keys: &[[u8; 32]], seq: u64, data: &[u8]) -> Result<Vec<u8>> {
    let nonce = cell_nonce(DIR_FWD, seq);
    let mut cur = frame_payload(data)?.to_vec();
    for k in keys.iter().rev() {
        cur = aead_seal(k, &nonce, &cur);
    }
    cur.resize(PAYLOAD_SIZE, 0);
    Ok(cur)
}

/// Legacy one-shot: build a fresh header + seal the payload at `seq = 0`. Bit-for-bit
/// identical to the original `create_onion` on the forward path (the forward seq-0 nonce
/// is all-zeros). Kept for the request_response path + existing tests during migration.
pub fn create_onion<R: RngCore + CryptoRng>(
    rng: &mut R,
    hops: &[OnionHop],
    payload: &[u8],
) -> Result<(Cell, Vec<[u8; 32]>)> {
    let (header, keys) = build_header(rng, hops)?;
    let delta = seal_forward_payload(&keys, 0, payload)?;
    Ok((
        Cell {
            alpha: header.alpha,
            beta: header.beta,
            gamma: header.gamma,
            delta,
        },
        keys,
    ))
}

/// Peel the routing HEADER once at a hop holding onion secret `y`: verify the header MAC,
/// learn the next hop (or exit destination), and derive this hop's `reply_key` + the
/// downstream header to forward. No payload is touched — call this once per circuit and
/// cache the result; then open each data cell's payload with [`peel_payload`].
pub fn peel_header(h: &Header, y: &Scalar) -> Result<PeeledHeader> {
    let alpha = CompressedRistretto(h.alpha)
        .decompress()
        .ok_or(NetError::Malformed("alpha"))?;
    let secret = (alpha * y).compress().to_bytes();

    // header integrity
    if header_mac(&secret, &h.beta) != h.gamma {
        return Err(NetError::OnionAuth(0));
    }

    // decrypt routing block + shift
    let mut padded = h.beta.clone();
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

    let reply_key = payload_key(&secret);
    if flags & FLAG_EXIT != 0 {
        Ok(PeeledHeader {
            addr,
            is_exit: true,
            reply_key,
            payload_len,
            fwd_header: None,
        })
    } else {
        let b = blinding_factor(&h.alpha, &secret);
        let alpha_out = (alpha * b).compress().to_bytes();
        Ok(PeeledHeader {
            addr,
            is_exit: false,
            reply_key,
            payload_len,
            fwd_header: Some(Header {
                alpha: alpha_out,
                beta: beta_next,
                gamma: next_mac,
            }),
        })
    }
}

/// Open one forward payload layer at a hop using its `reply_key` and the cell's `seq`.
/// Returns the inner bytes (the next hop's `delta`, or the exit's framed plaintext).
pub fn peel_payload(
    reply_key: &[u8; 32],
    seq: u64,
    payload_len: usize,
    delta: &[u8],
) -> Result<Vec<u8>> {
    if payload_len > delta.len() {
        return Err(NetError::Malformed("payload_len"));
    }
    aead_open(reply_key, &cell_nonce(DIR_FWD, seq), &delta[..payload_len])
}

/// Peel a full one-shot `Cell` (header + payload) at `seq` — header peel + payload peel.
pub fn peel_seq(cell: &Cell, y: &Scalar, seq: u64) -> Result<Peeled> {
    let header = Header {
        alpha: cell.alpha,
        beta: cell.beta.clone(),
        gamma: cell.gamma,
    };
    let ph = peel_header(&header, y)?;
    let inner = peel_payload(&ph.reply_key, seq, ph.payload_len, &cell.delta)?;
    if ph.is_exit {
        Ok(Peeled::Exit {
            dest: ph.addr,
            payload: unframe_payload(&inner)?,
            reply_key: ph.reply_key,
        })
    } else {
        let fwd = ph.fwd_header.expect("forward header present");
        let mut delta = inner;
        delta.resize(PAYLOAD_SIZE, 0);
        Ok(Peeled::Forward {
            next_addr: ph.addr,
            cell: Cell {
                alpha: fwd.alpha,
                beta: fwd.beta,
                gamma: fwd.gamma,
                delta,
            },
            reply_key: ph.reply_key,
        })
    }
}

/// Legacy one-shot peel at `seq = 0`. Kept for the request_response path + existing tests.
pub fn peel(cell: &Cell, y: &Scalar) -> Result<Peeled> {
    peel_seq(cell, y, 0)
}

/// Seal one reverse-path reply layer with a hop's `reply_key` and reverse `seq` (exit
/// seals the innermost layer; each hop toward the client adds an outer layer). The reverse
/// `seq` is the exit's own strictly-increasing per-circuit counter.
pub fn reply_seal_seq(reply_key: &[u8; 32], seq: u64, data: &[u8]) -> Vec<u8> {
    aead_seal(reply_key, &cell_nonce(DIR_REV, seq), data)
}

/// Open a fully-layered reply at the client at reverse `seq`. `reply_keys` are in forward
/// path order (entry first); the entry hop's layer is outermost, so we peel in path order.
pub fn reply_open_seq(reply_keys: &[[u8; 32]], seq: u64, sealed: &[u8]) -> Result<Vec<u8>> {
    let nonce = cell_nonce(DIR_REV, seq);
    let mut cur = sealed.to_vec();
    for k in reply_keys {
        cur = aead_open(k, &nonce, &cur)?;
    }
    Ok(cur)
}

/// Legacy reverse seal/open at `seq = 0`. Kept for the request_response path + tests.
pub fn reply_seal(reply_key: &[u8; 32], data: &[u8]) -> Vec<u8> {
    reply_seal_seq(reply_key, 0, data)
}
pub fn reply_open(reply_keys: &[[u8; 32]], sealed: &[u8]) -> Result<Vec<u8>> {
    reply_open_seq(reply_keys, 0, sealed)
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

    // ---- persistent-circuit crypto (build_header reused across many cells) ----

    #[test]
    fn header_reuse_carries_many_cells() {
        let mut rng = StdRng::seed_from_u64(123);
        let nodes: Vec<Node> = (0..3).map(|i| node(&mut rng, i as u8)).collect();
        let path = route(&nodes);

        // Build the routing header ONCE (the expensive ECDH chain).
        let (header, client_keys) = build_header(&mut rng, &path).unwrap();

        // The header peels identically every time — the property that makes reuse sound.
        let a = peel_header(&header, &nodes[0].secret).unwrap();
        let b = peel_header(&header, &nodes[0].secret).unwrap();
        assert_eq!(a.reply_key, b.reply_key);
        assert_eq!(a.addr, b.addr);

        // Stream many forward cells over the SAME header, each with an increasing seq.
        for seq in 0u64..6 {
            let msg = format!("cell number {seq}");
            let delta = seal_forward_payload(&client_keys, seq, msg.as_bytes()).unwrap();
            let mut cell = Cell {
                alpha: header.alpha,
                beta: header.beta.clone(),
                gamma: header.gamma,
                delta,
            };
            let mut hop_keys = Vec::new();
            let mut delivered = None;
            for (i, n) in nodes.iter().enumerate() {
                match peel_seq(&cell, &n.secret, seq).unwrap() {
                    Peeled::Forward {
                        next_addr,
                        cell: c,
                        reply_key,
                    } => {
                        assert_eq!(next_addr, nodes[i + 1].addr);
                        hop_keys.push(reply_key);
                        cell = c;
                    }
                    Peeled::Exit {
                        payload, reply_key, ..
                    } => {
                        hop_keys.push(reply_key);
                        delivered = Some(payload);
                    }
                }
            }
            assert_eq!(delivered.unwrap(), msg.as_bytes());
            assert_eq!(hop_keys, client_keys);

            // Reverse cell at its own seq, layered exit -> client.
            let reply = format!("reply to {seq}");
            let mut sealed = reply.clone().into_bytes();
            for k in hop_keys.iter().rev() {
                sealed = reply_seal_seq(k, seq, &sealed);
            }
            assert_eq!(
                reply_open_seq(&client_keys, seq, &sealed).unwrap(),
                reply.into_bytes()
            );
        }
    }

    #[test]
    fn distinct_seq_yields_distinct_ciphertext() {
        // The same payload at different seq MUST give different ciphertext (unique nonce) —
        // guards against the catastrophic key+nonce reuse that header reuse would otherwise
        // enable. seq=0 forward nonce is all-zeros (legacy compatibility).
        let mut rng = StdRng::seed_from_u64(5);
        let nodes: Vec<Node> = (0..3).map(|i| node(&mut rng, i as u8)).collect();
        let (_h, keys) = build_header(&mut rng, &route(&nodes)).unwrap();
        let a = seal_forward_payload(&keys, 0, b"same").unwrap();
        let b = seal_forward_payload(&keys, 1, b"same").unwrap();
        assert_ne!(a, b, "different seq must give different ciphertext");
    }
}
