//! WireGuard's link layer: a `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` handshake (the
//! exact WireGuard pattern) between two ADJACENT peers, via `snow`. `IK` = the
//! initiator already knows the responder's static x25519 key (learned from the DHT
//! descriptor); `psk2` mixes an optional pre-shared key into the second message. The
//! onion (Sphinx) packet rides INSIDE these transport messages — this layer only
//! provides hop-to-hop confidentiality + authenticity.

use snow::{Builder, HandshakeState, TransportState};

use crate::error::{NetError, Result};

const PARAMS: &str = "Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";
const PSK_INDEX: u8 = 2;

fn builder() -> Builder<'static> {
    Builder::new(PARAMS.parse().expect("static noise params parse"))
}

/// A live, post-handshake link session (transport mode). Each `write` produces a
/// ciphertext frame the peer feeds to `read`.
pub struct LinkSession {
    ts: TransportState,
}

impl LinkSession {
    pub fn write(&mut self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut out = vec![0u8; plaintext.len() + 64];
        let n = self.ts.write_message(plaintext, &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    pub fn read(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut out = vec![0u8; ciphertext.len()];
        let n = self.ts.read_message(ciphertext, &mut out)?;
        out.truncate(n);
        Ok(out)
    }
}

/// The initiating side of a link handshake.
pub struct HandshakeInitiator {
    hs: HandshakeState,
}

impl HandshakeInitiator {
    pub fn new(
        local_static: &[u8; 32],
        remote_static_pub: &[u8; 32],
        psk: Option<[u8; 32]>,
    ) -> Result<Self> {
        let hs = builder()
            .local_private_key(local_static)
            .remote_public_key(remote_static_pub)
            .psk(PSK_INDEX, &psk.unwrap_or([0u8; 32]))
            .build_initiator()?;
        Ok(Self { hs })
    }

    /// Produce handshake message 1 (`-> e, es, s, ss`).
    pub fn message(&mut self) -> Result<Vec<u8>> {
        let mut out = vec![0u8; 1024];
        let n = self.hs.write_message(&[], &mut out)?;
        out.truncate(n);
        Ok(out)
    }

    /// Read handshake message 2 (`<- e, ee, se, psk`) and enter transport mode.
    pub fn finish(mut self, msg2: &[u8]) -> Result<LinkSession> {
        let mut scratch = vec![0u8; msg2.len()];
        self.hs.read_message(msg2, &mut scratch)?;
        Ok(LinkSession {
            ts: self.hs.into_transport_mode()?,
        })
    }
}

/// The responding side of a link handshake.
pub struct HandshakeResponder {
    hs: HandshakeState,
}

impl HandshakeResponder {
    pub fn new(local_static: &[u8; 32], psk: Option<[u8; 32]>) -> Result<Self> {
        let hs = builder()
            .local_private_key(local_static)
            .psk(PSK_INDEX, &psk.unwrap_or([0u8; 32]))
            .build_responder()?;
        Ok(Self { hs })
    }

    /// Read message 1, produce message 2, and enter transport mode.
    pub fn respond(mut self, msg1: &[u8]) -> Result<(Vec<u8>, LinkSession)> {
        let mut scratch = vec![0u8; msg1.len()];
        self.hs.read_message(msg1, &mut scratch)?;
        let mut msg2 = vec![0u8; 1024];
        let n = self.hs.write_message(&[], &mut msg2)?;
        msg2.truncate(n);
        let ts = self.hs.into_transport_mode()?;
        Ok((msg2, LinkSession { ts }))
    }
}

/// Drive a full handshake in-memory (used by the simulator + tests).
pub fn handshake(
    initiator_static: &[u8; 32],
    responder_static: &[u8; 32],
    responder_public: &[u8; 32],
    psk: Option<[u8; 32]>,
) -> Result<(LinkSession, LinkSession)> {
    let mut init = HandshakeInitiator::new(initiator_static, responder_public, psk)?;
    let resp = HandshakeResponder::new(responder_static, psk)?;
    let msg1 = init.message()?;
    let (msg2, resp_session) = resp.respond(&msg1)?;
    let init_session = init.finish(&msg2)?;
    Ok((init_session, resp_session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use x25519_dalek::{PublicKey, StaticSecret};

    fn static_pair() -> ([u8; 32], [u8; 32]) {
        let s = StaticSecret::from([7u8; 32]);
        (s.to_bytes(), PublicKey::from(&s).to_bytes())
    }

    #[test]
    fn ik_handshake_and_transport_roundtrip() {
        let init_s = StaticSecret::from([3u8; 32]).to_bytes();
        let (resp_s, resp_pub) = static_pair();
        let (mut a, mut b) = handshake(&init_s, &resp_s, &resp_pub, None).unwrap();

        let ct = a.write(b"hello wireguard").unwrap();
        assert_eq!(b.read(&ct).unwrap(), b"hello wireguard");
        let ct2 = b.write(b"reply").unwrap();
        assert_eq!(a.read(&ct2).unwrap(), b"reply");
    }

    #[test]
    fn psk_must_match() {
        let init_s = StaticSecret::from([3u8; 32]).to_bytes();
        let (resp_s, resp_pub) = static_pair();
        // In IKpsk2 the PSK is mixed into message 2, so a mismatch surfaces when the
        // initiator reads msg2 (its key schedule diverges → AEAD tag fails), not when
        // the responder reads the psk-free msg1.
        let mut init = HandshakeInitiator::new(&init_s, &resp_pub, Some([9u8; 32])).unwrap();
        let resp = HandshakeResponder::new(&resp_s, Some([8u8; 32])).unwrap();
        let msg1 = init.message().unwrap();
        let (msg2, _resp_session) = resp.respond(&msg1).unwrap();
        assert!(init.finish(&msg2).is_err());
    }

    #[test]
    fn tampered_transport_frame_fails() {
        let init_s = StaticSecret::from([3u8; 32]).to_bytes();
        let (resp_s, resp_pub) = static_pair();
        let (mut a, mut b) = handshake(&init_s, &resp_s, &resp_pub, None).unwrap();
        let mut ct = a.write(b"secret").unwrap();
        ct[0] ^= 1;
        assert!(b.read(&ct).is_err());
    }
}
