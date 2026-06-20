//! Node identity: an ed25519 operator key (signs receipts + DHT descriptors, equals
//! the on-chain `operator` pubkey) and an x25519 static key (the WireGuard/Noise link
//! key, published in the DHT and committed via `NodeState.endpoint_hash`).

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rand::{CryptoRng, RngCore};
use x25519_dalek::{PublicKey, StaticSecret};

pub struct WeftKeypair {
    operator: SigningKey,
    static_secret: StaticSecret,
}

impl WeftKeypair {
    pub fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self {
        let mut sk = [0u8; 32];
        rng.fill_bytes(&mut sk);
        let operator = SigningKey::from_bytes(&sk);
        let mut xs = [0u8; 32];
        rng.fill_bytes(&mut xs);
        Self {
            operator,
            static_secret: StaticSecret::from(xs),
        }
    }

    /// Reconstruct from raw 32-byte seeds (operator ed25519 seed + x25519 static).
    pub fn from_bytes(operator_seed: &[u8; 32], static_secret: &[u8; 32]) -> Self {
        Self {
            operator: SigningKey::from_bytes(operator_seed),
            static_secret: StaticSecret::from(*static_secret),
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

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.operator.sign(msg).to_bytes()
    }
}

/// Verify an ed25519 signature against a 32-byte public key.
pub fn verify_sig(pubkey: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pubkey) else {
        return false;
    };
    vk.verify(msg, &ed25519_dalek::Signature::from_bytes(sig))
        .is_ok()
}
