//! Node identity, three keys:
//! - **operator** ed25519 — signs receipts + DHT descriptors, equals the on-chain
//!   `operator` pubkey;
//! - **static** x25519 — the WireGuard/Noise link key (hop-to-hop transport);
//! - **onion** Ristretto — the Sphinx onion key (prime-order group → clean blinding).
//!
//! All three are published in the DHT descriptor and committed via
//! `NodeState.endpoint_hash`.

use curve25519_dalek::{constants::RISTRETTO_BASEPOINT_POINT, scalar::Scalar};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::{CryptoRng, RngCore};
use x25519_dalek::{PublicKey, StaticSecret};

pub struct WeftKeypair {
    operator: SigningKey,
    static_secret: StaticSecret,
    onion_secret: Scalar,
}

impl WeftKeypair {
    pub fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let mut sk = [0u8; 32];
        rng.fill_bytes(&mut sk);
        let operator = SigningKey::from_bytes(&sk);
        let mut xs = [0u8; 32];
        rng.fill_bytes(&mut xs);
        let onion_secret = Scalar::random(rng);
        Self {
            operator,
            static_secret: StaticSecret::from(xs),
            onion_secret,
        }
    }

    /// Reconstruct from raw seeds (operator ed25519 seed, x25519 static, onion scalar).
    pub fn from_bytes(
        operator_seed: &[u8; 32],
        static_secret: &[u8; 32],
        onion_secret: &[u8; 32],
    ) -> Self {
        Self {
            operator: SigningKey::from_bytes(operator_seed),
            static_secret: StaticSecret::from(*static_secret),
            onion_secret: Scalar::from_bytes_mod_order(*onion_secret),
        }
    }

    /// The 32-byte ed25519 operator public key (== on-chain `operator`).
    pub fn operator_pubkey(&self) -> [u8; 32] {
        self.operator.verifying_key().to_bytes()
    }

    /// The 32-byte x25519 static public key for the Noise link layer.
    pub fn static_public(&self) -> [u8; 32] {
        PublicKey::from(&self.static_secret).to_bytes()
    }

    pub fn static_secret_bytes(&self) -> [u8; 32] {
        self.static_secret.to_bytes()
    }

    /// The compressed Ristretto onion public key for Sphinx.
    pub fn onion_public(&self) -> [u8; 32] {
        (RISTRETTO_BASEPOINT_POINT * self.onion_secret)
            .compress()
            .to_bytes()
    }

    pub fn onion_secret(&self) -> Scalar {
        self.onion_secret
    }

    /// The 32-byte onion (Ristretto) secret scalar, for persisting the node identity.
    pub fn onion_secret_bytes(&self) -> [u8; 32] {
        self.onion_secret.to_bytes()
    }

    /// The 32-byte ed25519 operator seed (private). Used to derive a stable libp2p
    /// identity so the node's `PeerId` is a deterministic function of its operator key.
    pub fn operator_seed(&self) -> [u8; 32] {
        self.operator.to_bytes()
    }

    /// A libp2p identity keypair derived deterministically from the operator key, so the
    /// resulting `PeerId` is reproducible by anyone who knows the operator pubkey (used
    /// to bind the DHT descriptor's `peer_id` to the on-chain operator identity).
    pub fn libp2p_keypair(&self) -> libp2p::identity::Keypair {
        let mut seed = self.operator_seed();
        libp2p::identity::Keypair::ed25519_from_bytes(&mut seed)
            .expect("32-byte ed25519 seed is always valid")
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.operator.sign(msg).to_bytes()
    }
}

/// The deterministic libp2p `PeerId` for an operator's ed25519 public key — what a
/// client checks a DHT descriptor's advertised `peer_id` against.
pub fn peer_id_for_operator(operator: &[u8; 32]) -> Option<libp2p::PeerId> {
    let pk = libp2p::identity::ed25519::PublicKey::try_from_bytes(operator).ok()?;
    Some(libp2p::identity::PublicKey::from(pk).to_peer_id())
}

/// Verify an ed25519 signature against a 32-byte public key.
pub fn verify_sig(pubkey: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pubkey) else {
        return false;
    };
    vk.verify(msg, &ed25519_dalek::Signature::from_bytes(sig))
        .is_ok()
}
