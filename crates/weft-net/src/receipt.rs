//! Dual-signed traffic receipts (the M6↔M4 seam). A receipt commits a relayed
//! byte-count for a `(client, operator, node)` over a 10-minute window; the client
//! and the relay each ed25519-sign the canonical 104-byte preimage
//! ([`weft_primitives::encode_receipt_core`]) — byte-identical to the aggregator's
//! `encodeReceiptCore`, so a receipt minted here verifies in the TS aggregator and
//! feeds the on-chain reward merkle. Serialized JSON matches the aggregator schema.

use serde::{Deserialize, Serialize};
use weft_primitives::encode_receipt_core;

use crate::keys::{verify_sig, WeftKeypair};

/// The signed fields of a receipt (raw 32-byte pubkeys; not yet base58/serde).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptCore {
    pub client: [u8; 32],
    pub operator: [u8; 32],
    pub node_id: u64,
    pub bytes: u64,
    pub window_start: u64,
    pub window_end: u64,
    pub nonce: u64,
}

impl ReceiptCore {
    pub fn encode(&self) -> [u8; 104] {
        encode_receipt_core(
            &self.client,
            &self.operator,
            self.node_id,
            self.bytes,
            self.window_start,
            self.window_end,
            self.nonce,
        )
    }

    /// Sign as the client (`client_sig`) — the operator key must equal `self.operator`.
    pub fn sign_client(&self, kp: &WeftKeypair) -> [u8; 64] {
        kp.sign(&self.encode())
    }

    /// Sign as the relay/operator (`relay_sig`).
    pub fn sign_relay(&self, kp: &WeftKeypair) -> [u8; 64] {
        kp.sign(&self.encode())
    }

    /// Both signatures must verify against the embedded client and operator keys.
    pub fn verify(&self, client_sig: &[u8; 64], relay_sig: &[u8; 64]) -> bool {
        let msg = self.encode();
        verify_sig(&self.client, &msg, client_sig) && verify_sig(&self.operator, &msg, relay_sig)
    }
}

/// The wire/JSON form, identical to the aggregator's `TrafficReceipt` (base58
/// addresses, decimal-string amounts, hex signatures).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrafficReceipt {
    pub client: String,
    pub operator: String,
    #[serde(rename = "nodeId")]
    pub node_id: String,
    pub bytes: String,
    #[serde(rename = "windowStart")]
    pub window_start: String,
    #[serde(rename = "windowEnd")]
    pub window_end: String,
    pub nonce: String,
    #[serde(rename = "clientSig")]
    pub client_sig: String,
    #[serde(rename = "relaySig")]
    pub relay_sig: String,
}

impl TrafficReceipt {
    pub fn new(core: &ReceiptCore, client_sig: &[u8; 64], relay_sig: &[u8; 64]) -> Self {
        Self {
            client: bs58::encode(core.client).into_string(),
            operator: bs58::encode(core.operator).into_string(),
            node_id: core.node_id.to_string(),
            bytes: core.bytes.to_string(),
            window_start: core.window_start.to_string(),
            window_end: core.window_end.to_string(),
            nonce: core.nonce.to_string(),
            client_sig: hex::encode(client_sig),
            relay_sig: hex::encode(relay_sig),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn dual_signed_receipt_verifies_and_serializes() {
        let mut rng = StdRng::seed_from_u64(1);
        let client = WeftKeypair::generate(&mut rng);
        let relay = WeftKeypair::generate(&mut rng);
        let core = ReceiptCore {
            client: client.operator_pubkey(),
            operator: relay.operator_pubkey(),
            node_id: 1,
            bytes: 2_000_000_000,
            window_start: 600,
            window_end: 1200,
            nonce: 1,
        };
        let cs = core.sign_client(&client);
        let rs = core.sign_relay(&relay);
        assert!(core.verify(&cs, &rs));
        // a forged relay sig (client signs both) fails
        let forged = core.sign_client(&client);
        assert!(!core.verify(&cs, &forged));

        let json = serde_json::to_string(&TrafficReceipt::new(&core, &cs, &rs)).unwrap();
        assert!(json.contains("\"nodeId\":\"1\""));
        assert!(json.contains("\"clientSig\""));
    }
}
