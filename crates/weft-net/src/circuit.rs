//! Persistent full-duplex circuit transport. A long-lived libp2p substream per hop-link
//! (`/weft/circuit/1.0.0`) carries many onion cells in BOTH directions, so a circuit is
//! built once (one Sphinx header) and the exit PUSHES return data — no per-cell substream
//! and no polling. This replaces the one-shot request/response `/weft/cell/1.0.0` path for
//! the VPN data plane; routing/privacy (Sphinx) and per-hop metering are unchanged.

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};
use std::io;

use crate::sphinx::Header;

/// The streaming circuit protocol (one persistent bidirectional substream per hop-link).
pub const CIRCUIT_PROTOCOL: StreamProtocol = StreamProtocol::new("/weft/circuit/1.0.0");

/// Upper bound on a single framed message (a reverse data chunk can be large).
pub const MAX_FRAME: usize = 512 * 1024;

/// A 16-byte circuit identifier the client picks at OPEN; each hop keys its per-circuit
/// state by it. (The substream handle is the actual routing handle; the id aids logging
/// and correlating the two halves at a hop.)
pub type CircuitId = [u8; 16];

/// One framed message on a circuit substream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Frame {
    /// Circuit setup, sent once as the first frame: the Sphinx header (reused for every
    /// cell) + the client-chosen circuit id.
    Open {
        circuit_id: CircuitId,
        header: Header,
    },
    /// A forward data cell: per-circuit forward `seq` (the AEAD nonce) + the onion payload.
    Fwd { seq: u64, delta: Vec<u8> },
    /// A pushed reverse data cell: the exit's per-circuit reverse `seq` + sealed reply bytes.
    Rev { seq: u64, sealed: Vec<u8> },
    /// Tear-down.
    Close,
}

/// Write one length-prefixed, bincode-encoded frame to a circuit substream.
pub async fn write_frame<S: AsyncWrite + Unpin>(s: &mut S, f: &Frame) -> io::Result<()> {
    let bytes =
        bincode::serialize(f).map_err(|e| io::Error::other(format!("frame encode: {e}")))?;
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::other("frame too large"));
    }
    s.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    s.write_all(&bytes).await?;
    s.flush().await?;
    Ok(())
}

/// Read one frame; `Ok(None)` on a clean end-of-stream (the peer closed the substream).
pub async fn read_frame<S: AsyncRead + Unpin>(s: &mut S) -> io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    match s.read_exact(&mut len).await {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::Error::other("frame too large"));
    }
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf).await?;
    let f =
        bincode::deserialize(&buf).map_err(|e| io::Error::other(format!("frame decode: {e}")))?;
    Ok(Some(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_roundtrip_over_a_stream() {
        let header = Header {
            alpha: [7u8; 32],
            beta: vec![1u8; crate::sphinx::BETA_LEN],
            gamma: [2u8; crate::sphinx::MAC_SIZE],
        };
        let frames = vec![
            Frame::Open {
                circuit_id: [9u8; 16],
                header,
            },
            Frame::Fwd {
                seq: 0,
                delta: vec![3u8; crate::sphinx::PAYLOAD_SIZE],
            },
            Frame::Fwd {
                seq: 1,
                delta: vec![4u8; crate::sphinx::PAYLOAD_SIZE],
            },
            Frame::Rev {
                seq: 0,
                sealed: b"hello back".to_vec(),
            },
            Frame::Close,
        ];

        // Write all frames into a buffer, then read them back in order.
        let mut buf = futures::io::Cursor::new(Vec::<u8>::new());
        for f in &frames {
            write_frame(&mut buf, f).await.unwrap();
        }
        buf.set_position(0);
        let mut got = Vec::new();
        while let Some(f) = read_frame(&mut buf).await.unwrap() {
            got.push(f);
        }
        assert_eq!(got, frames);
    }
}
