//! The Weft VPN stream protocol — a tiny framing carried *inside* each onion cell's
//! encrypted payload. The internet destination (`dst`) rides in this payload, so only the
//! exit node ever learns it; the 32-byte onion `dest` field only selects the exit. Each
//! forward frame fits one cell payload (`MAX_PAYLOAD`); the exit's reply is size-unbounded
//! (the reverse path only adds a 16-byte AEAD tag per hop), so downloads stream efficiently.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use weft_net::sphinx::MAX_PAYLOAD;

/// Forward-frame header overhead: kind(1) + stream_id(8) + seq(4).
const DATA_HEADER: usize = 13;
/// Max application bytes carried in a single forward `Data` frame (one onion cell).
pub const MAX_DATA_CHUNK: usize = MAX_PAYLOAD - DATA_HEADER;

const K_OPEN: u8 = 1;
const K_DATA: u8 = 2;
const K_CLOSE: u8 = 3;
const K_POLL: u8 = 4;

/// A frame the client sends toward the exit (one per onion cell).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientFrame {
    /// Open a new logical stream to `dst` (the real internet target). `udp` selects the
    /// transport at the exit: false → a TCP connection, true → a UDP socket (for DNS etc.).
    Open {
        stream_id: u64,
        dst: SocketAddr,
        udp: bool,
    },
    /// Stream payload bytes (already chunked to `MAX_DATA_CHUNK`).
    Data {
        stream_id: u64,
        seq: u32,
        data: Vec<u8>,
    },
    /// Tear the stream down.
    Close { stream_id: u64 },
    /// Carry no new data — just let the exit return whatever it has buffered to read
    /// (drains the server→client direction over the request/response transport).
    Poll { stream_id: u64 },
}

fn put_sockaddr(out: &mut Vec<u8>, addr: &SocketAddr) {
    match addr.ip() {
        IpAddr::V4(v4) => {
            out.push(4);
            out.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            out.push(6);
            out.extend_from_slice(&v6.octets());
        }
    }
    out.extend_from_slice(&addr.port().to_be_bytes());
}

fn get_sockaddr(buf: &[u8]) -> Option<SocketAddr> {
    let (&fam, rest) = buf.split_first()?;
    match fam {
        4 => {
            let ip: [u8; 4] = rest.get(0..4)?.try_into().ok()?;
            let port = u16::from_be_bytes(rest.get(4..6)?.try_into().ok()?);
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::from(ip)), port))
        }
        6 => {
            let ip: [u8; 16] = rest.get(0..16)?.try_into().ok()?;
            let port = u16::from_be_bytes(rest.get(16..18)?.try_into().ok()?);
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(ip)), port))
        }
        _ => None,
    }
}

impl ClientFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        match self {
            ClientFrame::Open {
                stream_id,
                dst,
                udp,
            } => {
                out.push(K_OPEN);
                out.extend_from_slice(&stream_id.to_le_bytes());
                out.push(*udp as u8);
                put_sockaddr(&mut out, dst);
            }
            ClientFrame::Data {
                stream_id,
                seq,
                data,
            } => {
                out.push(K_DATA);
                out.extend_from_slice(&stream_id.to_le_bytes());
                out.extend_from_slice(&seq.to_le_bytes());
                out.extend_from_slice(data);
            }
            ClientFrame::Close { stream_id } => {
                out.push(K_CLOSE);
                out.extend_from_slice(&stream_id.to_le_bytes());
            }
            ClientFrame::Poll { stream_id } => {
                out.push(K_POLL);
                out.extend_from_slice(&stream_id.to_le_bytes());
            }
        }
        out
    }

    pub fn decode(buf: &[u8]) -> Option<ClientFrame> {
        let (&kind, rest) = buf.split_first()?;
        let stream_id = u64::from_le_bytes(rest.get(0..8)?.try_into().ok()?);
        match kind {
            K_OPEN => Some(ClientFrame::Open {
                stream_id,
                udp: *rest.get(8)? != 0,
                dst: get_sockaddr(rest.get(9..)?)?,
            }),
            K_DATA => {
                let seq = u32::from_le_bytes(rest.get(8..12)?.try_into().ok()?);
                Some(ClientFrame::Data {
                    stream_id,
                    seq,
                    data: rest.get(12..)?.to_vec(),
                })
            }
            K_CLOSE => Some(ClientFrame::Close { stream_id }),
            K_POLL => Some(ClientFrame::Poll { stream_id }),
            _ => None,
        }
    }
}

const R_DATA: u8 = 1;
const R_EOF: u8 = 2;
const R_ERR: u8 = 3;

/// Exit error codes (in an `ExitFrame::Err`).
pub mod exit_err {
    pub const CONNECT_FAILED: u8 = 1;
    pub const POLICY_BLOCKED: u8 = 2;
    pub const NO_STREAM: u8 = 3;
    pub const IO: u8 = 4;
    pub const BAD_FRAME: u8 = 5;
}

/// The exit's reply (sealed back through the circuit as one cell response).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExitFrame {
    /// Bytes read from the real connection (may be empty on a `Poll` with nothing ready).
    Data(Vec<u8>),
    /// The upstream connection closed cleanly.
    Eof,
    /// An error (see [`exit_err`]).
    Err(u8),
}

impl ExitFrame {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ExitFrame::Data(d) => {
                let mut out = Vec::with_capacity(1 + d.len());
                out.push(R_DATA);
                out.extend_from_slice(d);
                out
            }
            ExitFrame::Eof => vec![R_EOF],
            ExitFrame::Err(code) => vec![R_ERR, *code],
        }
    }

    pub fn decode(buf: &[u8]) -> Option<ExitFrame> {
        let (&kind, rest) = buf.split_first()?;
        match kind {
            R_DATA => Some(ExitFrame::Data(rest.to_vec())),
            R_EOF => Some(ExitFrame::Eof),
            R_ERR => Some(ExitFrame::Err(*rest.first()?)),
            _ => None,
        }
    }
}

/// Split a byte buffer into back-to-back `Data` frames, each ≤ `MAX_DATA_CHUNK`.
pub fn chunk_data(stream_id: u64, start_seq: u32, data: &[u8]) -> Vec<ClientFrame> {
    if data.is_empty() {
        return Vec::new();
    }
    data.chunks(MAX_DATA_CHUNK)
        .enumerate()
        .map(|(i, c)| ClientFrame::Data {
            stream_id,
            seq: start_seq.wrapping_add(i as u32),
            data: c.to_vec(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_frames_round_trip() {
        let frames = [
            ClientFrame::Open {
                stream_id: 7,
                dst: "93.184.216.34:443".parse().unwrap(),
                udp: false,
            },
            ClientFrame::Open {
                stream_id: 9,
                dst: "[2606:2800:220:1:248:1893:25c8:1946]:80".parse().unwrap(),
                udp: true,
            },
            ClientFrame::Data {
                stream_id: 7,
                seq: 3,
                data: b"GET / HTTP/1.1\r\n\r\n".to_vec(),
            },
            ClientFrame::Close { stream_id: 7 },
            ClientFrame::Poll { stream_id: 7 },
        ];
        for f in frames {
            assert_eq!(ClientFrame::decode(&f.encode()).unwrap(), f);
        }
    }

    #[test]
    fn exit_frames_round_trip() {
        for f in [
            ExitFrame::Data(b"HTTP/1.1 200 OK".to_vec()),
            ExitFrame::Data(Vec::new()),
            ExitFrame::Eof,
            ExitFrame::Err(exit_err::CONNECT_FAILED),
        ] {
            assert_eq!(ExitFrame::decode(&f.encode()).unwrap(), f);
        }
    }

    #[test]
    fn data_chunks_fit_one_cell_and_reassemble() {
        let payload: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
        let frames = chunk_data(1, 0, &payload);
        assert!(frames.len() >= 5); // 5000 / 1009 ≈ 5
        let mut reassembled = Vec::new();
        for (i, f) in frames.iter().enumerate() {
            match f {
                ClientFrame::Data { seq, data, .. } => {
                    assert_eq!(*seq, i as u32);
                    assert!(data.len() <= MAX_DATA_CHUNK);
                    assert!(f.encode().len() <= MAX_PAYLOAD);
                    reassembled.extend_from_slice(data);
                }
                _ => panic!("expected Data"),
            }
        }
        assert_eq!(reassembled, payload);
    }

    #[test]
    fn truncated_frames_decode_to_none() {
        assert!(ClientFrame::decode(&[]).is_none());
        assert!(ClientFrame::decode(&[K_OPEN, 1, 2, 3]).is_none()); // short stream_id
        assert!(ExitFrame::decode(&[]).is_none());
    }
}
