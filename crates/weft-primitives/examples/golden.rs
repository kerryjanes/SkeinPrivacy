//! Emit the cross-language golden vectors consumed by the `@weft/sdk` parity
//! test (`sdk/test/math.parity.test.ts`). The TS reward/merkle mirror must
//! reproduce every output here byte-for-byte, so disputes are decidable and the
//! aggregator's amounts/leaves match exactly what the on-chain program accepts.
//!
//! Regenerate the committed fixture with:
//!   cargo run -q -p weft-primitives --example golden > sdk/src/__fixtures__/reward-vectors.json

use weft_primitives::{
    merkle::{hash_allocation_leaf, hash_reward_leaf, merkle_proof, merkle_root},
    split_payment, split_tge, traffic_reward, traffic_reward_with_bootstrap,
};

fn hex32(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    let mut out = String::new();
    out.push_str("{\n");

    // --- reward vectors (exercise every clamp + rounding path) ---
    let reward_inputs: &[(u64, u32, u32, u32)] = &[
        (1_000_000_000, 10_000, 0, 0),         // 1 GB, neutral
        (5_000_000_000, 20_000, 5_000, 2_000), // max rep/geo/staking
        (2_500_000_000, 3_000, 9_000, 9_000),  // rep below floor, geo+staking above cap
        (0, 20_000, 5_000, 2_000),             // zero bytes
        (123_456_789, 14_237, 1_234, 2_000),   // arbitrary
        (1_000_000_000_000, 12_000, 2_500, 0), // ~1 TB
    ];
    out.push_str("  \"rewards\": [\n");
    for (i, (bytes, rep, geo, stk)) in reward_inputs.iter().enumerate() {
        let reward = traffic_reward(*bytes, *rep, *geo, *stk);
        out.push_str(&format!(
            "    {{ \"bytes\": \"{bytes}\", \"reputationBps\": {rep}, \"geoBonusBps\": {geo}, \"stakingBonusBps\": {stk}, \"reward\": \"{reward}\" }}{}\n",
            if i + 1 < reward_inputs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n");

    // --- cold-start bootstrap-bonus vectors (M8) ---
    let bootstrap_inputs: &[(u64, u32, u32, u32, u32)] = &[
        (1_000_000_000, 10_000, 0, 0, 0),     // no bonus
        (1_000_000_000, 10_000, 0, 0, 5_000), // +50%
        (1_000_000_000, 20_000, 5_000, 2_000, 5_000),
        (1_000_000_000, 10_000, 0, 0, 99_999), // clamped to +100%
    ];
    out.push_str("  \"bootstrap\": [\n");
    for (i, (bytes, rep, geo, stk, boot)) in bootstrap_inputs.iter().enumerate() {
        let reward = traffic_reward_with_bootstrap(*bytes, *rep, *geo, *stk, *boot);
        out.push_str(&format!(
            "    {{ \"bytes\": \"{bytes}\", \"reputationBps\": {rep}, \"geoBonusBps\": {geo}, \"stakingBonusBps\": {stk}, \"bootstrapBonusBps\": {boot}, \"reward\": \"{reward}\" }}{}\n",
            if i + 1 < bootstrap_inputs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n");

    // --- IDO TGE-split vectors (M8) ---
    let tge_inputs: &[(u64, u32)] = &[
        (150_000_000_000_000, 2_500), // 150M WEFT @ 25% (6 decimals)
        (1_000_000, 2_500),
        (3, 2_500),
        (1_000, 10_000), // 100% at TGE
    ];
    out.push_str("  \"tge\": [\n");
    for (i, (allocation, tge_bps)) in tge_inputs.iter().enumerate() {
        let (tge, vest) = split_tge(*allocation, *tge_bps);
        out.push_str(&format!(
            "    {{ \"allocation\": \"{allocation}\", \"tgeBps\": {tge_bps}, \"tge\": \"{tge}\", \"vesting\": \"{vest}\" }}{}\n",
            if i + 1 < tge_inputs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n");

    // --- allocation-leaf vectors (distributor-bound, domain 0x02) ---
    let alloc_inputs: &[([u8; 32], [u8; 32], u64)] = &[
        ([1u8; 32], [2u8; 32], 1_000_000),
        ([3u8; 32], [4u8; 32], u64::MAX),
    ];
    out.push_str("  \"allocLeaves\": [\n");
    for (i, (dist, claimant, amount)) in alloc_inputs.iter().enumerate() {
        let leaf = hash_allocation_leaf(dist, claimant, *amount);
        out.push_str(&format!(
            "    {{ \"distributor\": \"{}\", \"claimant\": \"{}\", \"amount\": \"{amount}\", \"leaf\": \"{}\" }}{}\n",
            hex32(dist),
            hex32(claimant),
            hex32(&leaf),
            if i + 1 < alloc_inputs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n");

    // --- payment-split vectors (70/20/10 with rounding remainder) ---
    let split_inputs: &[u64] = &[1_000_000, 7, 1, 0, 999_999_999, 123_456_789];
    out.push_str("  \"splits\": [\n");
    for (i, amount) in split_inputs.iter().enumerate() {
        let s = split_payment(*amount);
        out.push_str(&format!(
            "    {{ \"amount\": \"{amount}\", \"nodes\": \"{}\", \"burn\": \"{}\", \"treasury\": \"{}\" }}{}\n",
            s.nodes,
            s.burn,
            s.treasury,
            if i + 1 < split_inputs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n");

    // --- leaf vectors (epoch-bound, domain-separated) ---
    let leaf_inputs: &[(u64, [u8; 32], u64, u64)] = &[
        (42, [2u8; 32], 7, 123_456),
        (1, [0u8; 32], 0, 1),
        (600, [255u8; 32], u64::MAX, u64::MAX),
        (7, [9u8; 32], 10, 300_000),
    ];
    out.push_str("  \"leaves\": [\n");
    for (i, (epoch, op, id, amt)) in leaf_inputs.iter().enumerate() {
        let leaf = hash_reward_leaf(*epoch, op, *id, *amt);
        out.push_str(&format!(
            "    {{ \"epoch\": {epoch}, \"operator\": \"{}\", \"nodeId\": \"{id}\", \"amount\": \"{amt}\", \"leaf\": \"{}\" }}{}\n",
            hex32(op),
            hex32(&leaf),
            if i + 1 < leaf_inputs.len() { "," } else { "" }
        ));
    }
    out.push_str("  ],\n");

    // --- a full tree with root + every proof (odd leaf count → odd-promote path) ---
    let epoch = 7u64;
    let entries: &[([u8; 32], u64, u64)] = &[
        ([10u8; 32], 10, 300_000),
        ([11u8; 32], 11, 200_000),
        ([12u8; 32], 12, 150_000),
        ([13u8; 32], 13, 50_000),
        ([14u8; 32], 14, 25_000),
    ];
    let leaves: Vec<[u8; 32]> = entries
        .iter()
        .map(|(op, id, amt)| hash_reward_leaf(epoch, op, *id, *amt))
        .collect();
    let root = merkle_root(&leaves);
    out.push_str("  \"tree\": {\n");
    out.push_str(&format!("    \"epoch\": {epoch},\n"));
    out.push_str(&format!("    \"root\": \"{}\",\n", hex32(&root)));
    out.push_str("    \"entries\": [\n");
    for (i, (op, id, amt)) in entries.iter().enumerate() {
        let proof = merkle_proof(&leaves, i);
        let proof_str: Vec<String> = proof.iter().map(|p| format!("\"{}\"", hex32(p))).collect();
        out.push_str(&format!(
            "      {{ \"operator\": \"{}\", \"nodeId\": \"{id}\", \"amount\": \"{amt}\", \"proof\": [{}] }}{}\n",
            hex32(op),
            proof_str.join(", "),
            if i + 1 < entries.len() { "," } else { "" }
        ));
    }
    out.push_str("    ]\n");
    out.push_str("  }\n");

    out.push_str("}\n");
    print!("{out}");
}
