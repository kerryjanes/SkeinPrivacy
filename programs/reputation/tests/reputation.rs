//! Multi-program LiteSVM tests: load node-registry + reputation, prove the
//! reputation update mirrors a multiplier into NodeState, plus EMA/auth/penalize.

#![allow(clippy::result_large_err)]

use {
    anchor_lang::{
        solana_program::instruction::Instruction, AccountDeserialize, AccountSerialize,
        InstructionData, ToAccountMetas,
    },
    litesvm::{types::TransactionResult, LiteSVM},
    solana_account::Account,
    solana_clock::Clock,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_pubkey::Pubkey,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const NODE_REGISTRY_BYTES: &[u8] = include_bytes!("../../../target/deploy/node_registry.so");
const REPUTATION_BYTES: &[u8] = include_bytes!("../../../target/deploy/reputation.so");

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(node_registry::ID, NODE_REGISTRY_BYTES)
        .unwrap();
    svm.add_program(reputation::ID, REPUTATION_BYTES).unwrap();
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();
    (svm, payer)
}
fn set_clock(svm: &mut LiteSVM, ts: i64) {
    let mut c: Clock = svm.get_sysvar();
    c.unix_timestamp = ts;
    svm.set_sysvar(&c);
}
fn send(
    svm: &mut LiteSVM,
    ix: Instruction,
    payer: &Keypair,
    signers: &[&Keypair],
) -> TransactionResult {
    svm.expire_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
}
fn assert_failed_with(res: TransactionResult, code: &str) {
    let f = res.expect_err("expected failure");
    let logs = f.meta.logs.join("\n");
    assert!(logs.contains(code), "expected `{code}`, got:\n{logs}");
}

fn registry_pda() -> Pubkey {
    Pubkey::find_program_address(&[node_registry::REGISTRY_SEED], &node_registry::ID).0
}
fn node_pda(op: &Pubkey, id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[node_registry::NODE_SEED, op.as_ref(), &id.to_le_bytes()],
        &node_registry::ID,
    )
    .0
}
fn config_pda() -> Pubkey {
    Pubkey::find_program_address(&[reputation::CONFIG_SEED], &reputation::ID).0
}
fn state_pda(op: &Pubkey, id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[reputation::STATE_SEED, op.as_ref(), &id.to_le_bytes()],
        &reputation::ID,
    )
    .0
}
fn rep_authority_pda() -> Pubkey {
    Pubkey::find_program_address(&[reputation::AUTHORITY_SEED], &reputation::ID).0
}

fn init_registry(svm: &mut LiteSVM, authority: &Keypair) {
    let data = node_registry::instruction::InitializeRegistry {}.data();
    let metas = node_registry::accounts::InitializeRegistry {
        authority: authority.pubkey(),
        collection: Pubkey::new_unique(),
        active_tree: Pubkey::new_unique(),
        registry: registry_pda(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(node_registry::ID, &data, metas),
        authority,
        &[authority],
    )
    .unwrap();
    let data = node_registry::instruction::SetMetricsAuthorities {
        reputation_authority: rep_authority_pda(),
        staking_authority: Pubkey::new_unique(),
    }
    .data();
    let metas = node_registry::accounts::AdminRegistry {
        authority: authority.pubkey(),
        registry: registry_pda(),
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(node_registry::ID, &data, metas),
        authority,
        &[authority],
    )
    .unwrap();
}
fn craft_node(svm: &mut LiteSVM, operator: &Pubkey, node_id: u64) -> Pubkey {
    let pda = node_pda(operator, node_id);
    let bump = Pubkey::find_program_address(
        &[
            node_registry::NODE_SEED,
            operator.as_ref(),
            &node_id.to_le_bytes(),
        ],
        &node_registry::ID,
    )
    .1;
    let node = node_registry::NodeState {
        operator: *operator,
        node_id,
        asset_id: Pubkey::new_unique(),
        merkle_tree: Pubkey::new_unique(),
        leaf_nonce: 0,
        geo: 1,
        capabilities: 1,
        endpoint_hash: [0; 32],
        availability: 50,
        status: node_registry::STATUS_ACTIVE,
        registered_at: 1,
        updated_at: 1,
        reputation: 0,
        stake_amount: 0,
        bump,
    };
    let mut data = Vec::new();
    node.try_serialize(&mut data).unwrap();
    svm.set_account(
        pda,
        Account {
            lamports: 10_000_000,
            data,
            owner: node_registry::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
    pda
}
fn node_reputation(svm: &LiteSVM, node: &Pubkey) -> u16 {
    node_registry::NodeState::try_deserialize(&mut svm.get_account(node).unwrap().data.as_slice())
        .unwrap()
        .reputation
}

fn init_config(svm: &mut LiteSVM, authority: &Keypair, oracle: &Pubkey) {
    let data = reputation::instruction::InitializeConfig {}.data();
    let metas = reputation::accounts::InitializeConfig {
        authority: authority.pubkey(),
        oracle: *oracle,
        config: config_pda(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(reputation::ID, &data, metas),
        authority,
        &[authority],
    )
    .unwrap();
}
#[allow(clippy::too_many_arguments)]
fn update_ix(
    oracle: &Pubkey,
    operator: &Pubkey,
    node_id: u64,
    up: u32,
    sp: u32,
    rv: u32,
    node: Option<Pubkey>,
) -> Instruction {
    let data = reputation::instruction::UpdateMetrics {
        node_id,
        uptime_bps: up,
        speed_bps: sp,
        review_bps: rv,
    }
    .data();
    let metas = reputation::accounts::UpdateMetrics {
        oracle: *oracle,
        config: config_pda(),
        operator: *operator,
        state: state_pda(operator, node_id),
        program_authority: rep_authority_pda(),
        node_registry_program: node_registry::ID,
        registry: registry_pda(),
        node,
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(reputation::ID, &data, metas)
}
fn get_state(svm: &LiteSVM, op: &Pubkey, id: u64) -> reputation::ReputationState {
    reputation::ReputationState::try_deserialize(
        &mut svm.get_account(&state_pda(op, id)).unwrap().data.as_slice(),
    )
    .unwrap()
}

#[test]
fn update_metrics_mirrors_multiplier() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, 1_000);
    init_registry(&mut svm, &authority);
    let oracle = Keypair::new();
    svm.airdrop(&oracle.pubkey(), 1_000_000_000_000).unwrap();
    init_config(&mut svm, &authority, &oracle.pubkey());
    let op = Pubkey::new_unique();
    let node = craft_node(&mut svm, &op, 1);

    // perfect signals; first update seeds baseline then EMAs the sample
    send(
        &mut svm,
        update_ix(&oracle.pubkey(), &op, 1, 10_000, 10_000, 10_000, Some(node)),
        &oracle,
        &[&oracle],
    )
    .unwrap();

    let st = get_state(&svm, &op, 1);
    let expected_quality = weft_primitives::reputation_ema(
        weft_primitives::REPUTATION_BASELINE_QUALITY_BPS,
        10_000,
        weft_primitives::REPUTATION_EMA_ALPHA_BPS,
    );
    assert_eq!(st.quality_bps, expected_quality);
    let expected_mult = weft_primitives::reputation_multiplier_bps(expected_quality);
    assert_eq!(st.multiplier_bps, expected_mult);
    assert_eq!(node_reputation(&svm, &node) as u32, expected_mult); // mirrored!
    assert!(expected_mult > 10_000); // good signals → above 1.0x
}

#[test]
fn ema_smooths_across_updates() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, 1_000);
    init_registry(&mut svm, &authority);
    let oracle = Keypair::new();
    svm.airdrop(&oracle.pubkey(), 1_000_000_000_000).unwrap();
    init_config(&mut svm, &authority, &oracle.pubkey());
    let op = Pubkey::new_unique();
    let node = craft_node(&mut svm, &op, 1);

    send(
        &mut svm,
        update_ix(&oracle.pubkey(), &op, 1, 10_000, 10_000, 10_000, Some(node)),
        &oracle,
        &[&oracle],
    )
    .unwrap();
    let q1 = get_state(&svm, &op, 1).quality_bps;
    // second update, same slot → no decay; EMA blends again
    send(
        &mut svm,
        update_ix(&oracle.pubkey(), &op, 1, 10_000, 10_000, 10_000, Some(node)),
        &oracle,
        &[&oracle],
    )
    .unwrap();
    let q2 = get_state(&svm, &op, 1).quality_bps;
    assert_eq!(
        q2,
        weft_primitives::reputation_ema(q1, 10_000, weft_primitives::REPUTATION_EMA_ALPHA_BPS)
    );
    assert!(q2 > q1); // converging upward toward the high sample
}

#[test]
fn rejects_non_oracle_and_bad_metrics() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, 1_000);
    init_registry(&mut svm, &authority);
    let oracle = Keypair::new();
    svm.airdrop(&oracle.pubkey(), 1_000_000_000_000).unwrap();
    init_config(&mut svm, &authority, &oracle.pubkey());
    let op = Pubkey::new_unique();
    let node = craft_node(&mut svm, &op, 1);

    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), 1_000_000_000_000).unwrap();
    assert_failed_with(
        send(
            &mut svm,
            update_ix(&attacker.pubkey(), &op, 1, 5_000, 5_000, 5_000, Some(node)),
            &attacker,
            &[&attacker],
        ),
        "Unauthorized",
    );
    assert_failed_with(
        send(
            &mut svm,
            update_ix(&oracle.pubkey(), &op, 1, 10_001, 5_000, 5_000, Some(node)),
            &oracle,
            &[&oracle],
        ),
        "InvalidMetric",
    );
}

#[test]
fn penalize_lowers_and_mirrors() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, 1_000);
    init_registry(&mut svm, &authority);
    let oracle = Keypair::new();
    svm.airdrop(&oracle.pubkey(), 1_000_000_000_000).unwrap();
    init_config(&mut svm, &authority, &oracle.pubkey());
    let op = Pubkey::new_unique();
    let node = craft_node(&mut svm, &op, 1);
    send(
        &mut svm,
        update_ix(&oracle.pubkey(), &op, 1, 10_000, 10_000, 10_000, Some(node)),
        &oracle,
        &[&oracle],
    )
    .unwrap();
    let before = node_reputation(&svm, &node);

    let data = reputation::instruction::Penalize {
        severity_bps: 5_000,
    }
    .data();
    let metas = reputation::accounts::Penalize {
        oracle: oracle.pubkey(),
        config: config_pda(),
        state: state_pda(&op, 1),
        program_authority: rep_authority_pda(),
        node_registry_program: node_registry::ID,
        registry: registry_pda(),
        node: Some(node),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(reputation::ID, &data, metas),
        &oracle,
        &[&oracle],
    )
    .unwrap();
    assert!(node_reputation(&svm, &node) < before); // penalized
}
