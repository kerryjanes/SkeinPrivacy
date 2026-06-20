//! Emit the cross-language receipt golden vectors consumed by the aggregator's
//! `receipt-parity` + `m6-ingest` vitests. `parity` fixes the 104-byte
//! `encode_receipt_core` layout the TS `encodeReceiptCore` must reproduce; `signed`
//! is a set of fully dual-signed `TrafficReceipt`s the TS aggregator `buildEpoch`
//! must accept — proving a Rust-minted receipt flows into the live M4 settlement.
//!
//! Regenerate:
//!   cargo run -q -p weft-net --example receipt_golden > services/aggregator/test/__fixtures__/receipt-vectors.json

use rand::rngs::StdRng;
use rand::SeedableRng;
use weft_net::keys::WeftKeypair;
use weft_net::receipt::{ReceiptCore, TrafficReceipt};
use weft_primitives::encode_receipt_core;

fn b58(b: &[u8; 32]) -> String {
    bs58::encode(b).into_string()
}
fn hex104(b: &[u8; 104]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    let mut rng = StdRng::seed_from_u64(0x5ce1_6601);

    // --- parity vectors: deterministic byte inputs → core hex ---
    // (client, operator, node_id, bytes, window_start, window_end, nonce)
    type Vector = ([u8; 32], [u8; 32], u64, u64, u64, u64, u64);
    let parity: &[Vector] = &[
        ([1u8; 32], [2u8; 32], 7, 123_456, 600, 1200, 1),
        ([0u8; 32], [255u8; 32], 0, 1, 0, 600, 0),
        ([9u8; 32], [3u8; 32], u64::MAX, u64::MAX, 600, 601, u64::MAX),
    ];
    let mut out = String::from("{\n  \"parity\": [\n");
    for (i, (c, o, id, by, ws, we, n)) in parity.iter().enumerate() {
        let core = encode_receipt_core(c, o, *id, *by, *ws, *we, *n);
        out.push_str(&format!(
            "    {{ \"client\": \"{}\", \"operator\": \"{}\", \"nodeId\": \"{id}\", \"bytes\": \"{by}\", \"windowStart\": \"{ws}\", \"windowEnd\": \"{we}\", \"nonce\": \"{n}\", \"core\": \"{}\" }}{}\n",
            b58(c), b58(o), hex104(&core),
            if i + 1 < parity.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n  \"signed\": [\n");

    // --- signed receipts: two real dual-signed receipts for epoch 1 ([600,1200)) ---
    let client = WeftKeypair::generate(&mut rng);
    let op_a = WeftKeypair::generate(&mut rng);
    let op_b = WeftKeypair::generate(&mut rng);
    let signed_specs = [
        (&op_a, 1u64, 2_000_000_000u64, 1u64),
        (&op_b, 2, 5_000_000_000, 1),
    ];
    for (i, (op, node_id, bytes, nonce)) in signed_specs.iter().enumerate() {
        let core = ReceiptCore {
            client: client.operator_pubkey(),
            operator: op.operator_pubkey(),
            node_id: *node_id,
            bytes: *bytes,
            window_start: 600,
            window_end: 1200,
            nonce: *nonce,
        };
        let cs = core.sign_client(&client);
        let rs = core.sign_relay(op);
        let r = TrafficReceipt::new(&core, &cs, &rs);
        out.push_str(&format!(
            "    {{ \"client\": \"{}\", \"operator\": \"{}\", \"nodeId\": \"{}\", \"bytes\": \"{}\", \"windowStart\": \"{}\", \"windowEnd\": \"{}\", \"nonce\": \"{}\", \"clientSig\": \"{}\", \"relaySig\": \"{}\" }}{}\n",
            r.client, r.operator, r.node_id, r.bytes, r.window_start, r.window_end, r.nonce, r.client_sig, r.relay_sig,
            if i + 1 < signed_specs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}\n");
    print!("{out}");
}
