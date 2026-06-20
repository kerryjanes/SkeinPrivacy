//! LiteSVM tests for the node-registry admin/lifecycle surface (no cNFT CPI).

#![allow(clippy::result_large_err)]

use {
    anchor_lang::{
        solana_program::instruction::Instruction, AccountDeserialize, InstructionData,
        ToAccountMetas,
    },
    litesvm::{types::TransactionResult, LiteSVM},
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_pubkey::Pubkey,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const PROGRAM_BYTES: &[u8] = include_bytes!("../../../target/deploy/node_registry.so");

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(node_registry::ID, PROGRAM_BYTES).unwrap();
    let authority = Keypair::new();
    svm.airdrop(&authority.pubkey(), 100_000_000_000).unwrap();
    (svm, authority)
}

fn registry_pda() -> Pubkey {
    Pubkey::find_program_address(&[node_registry::REGISTRY_SEED], &node_registry::ID).0
}

fn tree_shard_pda(index: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[node_registry::TREE_SEED, &index.to_le_bytes()],
        &node_registry::ID,
    )
    .0
}

fn send(svm: &mut LiteSVM, ix: Instruction, payer: &Keypair) -> TransactionResult {
    svm.expire_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer]).unwrap();
    svm.send_transaction(tx)
}

fn assert_failed_with(res: TransactionResult, code: &str) {
    let failed = res.expect_err("expected the transaction to fail");
    let logs = failed.meta.logs.join("\n");
    assert!(logs.contains(code), "expected `{code}`, got:\n{logs}");
}

fn get_registry(svm: &LiteSVM) -> node_registry::Registry {
    let acc = svm.get_account(&registry_pda()).unwrap();
    node_registry::Registry::try_deserialize(&mut acc.data.as_slice()).unwrap()
}

fn get_tree_shard(svm: &LiteSVM, index: u16) -> node_registry::TreeShard {
    let acc = svm.get_account(&tree_shard_pda(index)).unwrap();
    node_registry::TreeShard::try_deserialize(&mut acc.data.as_slice()).unwrap()
}

fn init_ix(authority: &Pubkey, collection: &Pubkey, active_tree: &Pubkey) -> Instruction {
    let data = node_registry::instruction::InitializeRegistry {}.data();
    let metas = node_registry::accounts::InitializeRegistry {
        authority: *authority,
        collection: *collection,
        active_tree: *active_tree,
        registry: registry_pda(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(node_registry::ID, &data, metas)
}

fn register_tree_ix(
    authority: &Pubkey,
    merkle_tree: &Pubkey,
    index: u16,
    max_depth: u32,
) -> Instruction {
    let data = node_registry::instruction::RegisterTree { index, max_depth }.data();
    let metas = node_registry::accounts::RegisterTree {
        authority: *authority,
        registry: registry_pda(),
        merkle_tree: *merkle_tree,
        tree_shard: tree_shard_pda(index),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(node_registry::ID, &data, metas)
}

fn init_registry(svm: &mut LiteSVM, authority: &Keypair) -> (Pubkey, Pubkey) {
    let collection = Pubkey::new_unique();
    let tree = Pubkey::new_unique();
    send(
        svm,
        init_ix(&authority.pubkey(), &collection, &tree),
        authority,
    )
    .unwrap();
    (collection, tree)
}

#[test]
fn initialize_sets_fields() {
    let (mut svm, authority) = setup();
    let (collection, tree) = init_registry(&mut svm, &authority);
    let r = get_registry(&svm);
    assert_eq!(r.authority, authority.pubkey());
    assert_eq!(r.collection, collection);
    assert_eq!(r.active_tree, tree);
    assert_eq!(r.tree_count, 0);
    assert_eq!(r.node_count, 0);
    assert_eq!(r.reputation_authority, authority.pubkey());
    assert_eq!(r.staking_authority, authority.pubkey());
    assert!(!r.paused);
}

#[test]
fn register_tree_sequential() {
    let (mut svm, authority) = setup();
    init_registry(&mut svm, &authority);

    let t0 = Pubkey::new_unique();
    send(
        &mut svm,
        register_tree_ix(&authority.pubkey(), &t0, 0, 20),
        &authority,
    )
    .unwrap();
    let shard = get_tree_shard(&svm, 0);
    assert_eq!(shard.merkle_tree, t0);
    assert_eq!(shard.capacity, 1 << 20);
    assert_eq!(shard.minted, 0);
    let r = get_registry(&svm);
    assert_eq!(r.tree_count, 1);
    assert_eq!(r.active_tree, t0);

    // Next shard must be index 1.
    let t1 = Pubkey::new_unique();
    send(
        &mut svm,
        register_tree_ix(&authority.pubkey(), &t1, 1, 20),
        &authority,
    )
    .unwrap();
    assert_eq!(get_registry(&svm).tree_count, 2);
    assert_eq!(get_registry(&svm).active_tree, t1);
}

#[test]
fn register_tree_rejects_nonsequential_index() {
    let (mut svm, authority) = setup();
    init_registry(&mut svm, &authority);
    let t = Pubkey::new_unique();
    // index 1 while tree_count == 0
    let res = send(
        &mut svm,
        register_tree_ix(&authority.pubkey(), &t, 1, 20),
        &authority,
    );
    assert_failed_with(res, "TreeIndexMismatch");
}

#[test]
fn register_tree_rejects_bad_depth() {
    let (mut svm, authority) = setup();
    init_registry(&mut svm, &authority);
    let t = Pubkey::new_unique();
    assert_failed_with(
        send(
            &mut svm,
            register_tree_ix(&authority.pubkey(), &t, 0, 0),
            &authority,
        ),
        "InvalidTree",
    );
    assert_failed_with(
        send(
            &mut svm,
            register_tree_ix(&authority.pubkey(), &t, 0, 31),
            &authority,
        ),
        "InvalidTree",
    );
}

#[test]
fn register_tree_rejects_non_authority() {
    let (mut svm, authority) = setup();
    init_registry(&mut svm, &authority);
    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), 100_000_000_000).unwrap();
    let t = Pubkey::new_unique();
    let res = send(
        &mut svm,
        register_tree_ix(&attacker.pubkey(), &t, 0, 20),
        &attacker,
    );
    assert_failed_with(res, "Unauthorized");
}

#[test]
fn set_authority_and_paused() {
    let (mut svm, authority) = setup();
    init_registry(&mut svm, &authority);

    // set_paused
    let data = node_registry::instruction::SetPaused { paused: true }.data();
    let metas = node_registry::accounts::AdminRegistry {
        authority: authority.pubkey(),
        registry: registry_pda(),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(node_registry::ID, &data, metas),
        &authority,
    )
    .unwrap();
    assert!(get_registry(&svm).paused);

    // set_authority to a new key
    let new_auth = Keypair::new();
    let data = node_registry::instruction::SetAuthority {
        new_authority: new_auth.pubkey(),
    }
    .data();
    let metas = node_registry::accounts::AdminRegistry {
        authority: authority.pubkey(),
        registry: registry_pda(),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(node_registry::ID, &data, metas),
        &authority,
    )
    .unwrap();
    assert_eq!(get_registry(&svm).authority, new_auth.pubkey());

    // old authority can no longer act
    let data = node_registry::instruction::SetPaused { paused: false }.data();
    let metas = node_registry::accounts::AdminRegistry {
        authority: authority.pubkey(),
        registry: registry_pda(),
    }
    .to_account_metas(None);
    let res = send(
        &mut svm,
        Instruction::new_with_bytes(node_registry::ID, &data, metas),
        &authority,
    );
    assert_failed_with(res, "Unauthorized");
}

// --- register (mintV2) validation + update lifecycle ---

use {anchor_lang::AccountSerialize, solana_account::Account};

const WIREGUARD: u32 = 1; // weft_primitives::capability::WIREGUARD
fn bubblegum_id() -> Pubkey {
    Pubkey::from_str_const("BGUMAp9Gq7iTEuizy4pqaxsTyUCBK68MDfK752saRPUY")
}

fn node_pda(operator: &Pubkey, node_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            node_registry::NODE_SEED,
            operator.as_ref(),
            &node_id.to_le_bytes(),
        ],
        &node_registry::ID,
    )
    .0
}

#[allow(clippy::too_many_arguments)]
fn register_ix(
    operator: &Pubkey,
    collection: &Pubkey,
    active_tree: &Pubkey,
    node_id: u64,
    geo: u32,
    capabilities: u32,
    availability: u8,
) -> Instruction {
    let data = node_registry::instruction::Register {
        node_id,
        geo,
        capabilities,
        endpoint_hash: [0u8; 32],
        availability,
        metadata_uri: "https://weft.network/n.json".to_string(),
    }
    .data();
    let metas = node_registry::accounts::Register {
        operator: *operator,
        registry: registry_pda(),
        tree_shard: tree_shard_pda(0),
        node: node_pda(operator, node_id),
        tree_config: Pubkey::new_unique(),
        merkle_tree: *active_tree,
        core_collection: *collection,
        mpl_core_cpi_signer: Pubkey::new_unique(),
        log_wrapper: Pubkey::new_unique(),
        compression_program: Pubkey::new_unique(),
        mpl_core_program: Pubkey::new_unique(),
        bubblegum_program: bubblegum_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(node_registry::ID, &data, metas)
}

/// Registry + one tree shard, returning (collection, active_tree).
fn setup_with_tree(svm: &mut LiteSVM, authority: &Keypair) -> (Pubkey, Pubkey) {
    let (collection, _tree) = init_registry(svm, authority);
    let merkle = Pubkey::new_unique();
    send(
        svm,
        register_tree_ix(&authority.pubkey(), &merkle, 0, 20),
        authority,
    )
    .unwrap();
    (collection, merkle)
}

fn set_node_state(svm: &mut LiteSVM, operator: &Pubkey, node_id: u64, status: u8) -> Pubkey {
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
        geo: 100,
        capabilities: WIREGUARD,
        endpoint_hash: [0u8; 32],
        availability: 50,
        status,
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

fn get_node(svm: &LiteSVM, pda: &Pubkey) -> node_registry::NodeState {
    let acc = svm.get_account(pda).unwrap();
    node_registry::NodeState::try_deserialize(&mut acc.data.as_slice()).unwrap()
}

#[test]
fn register_rejects_invalid_inputs() {
    let (mut svm, authority) = setup();
    let (collection, tree) = setup_with_tree(&mut svm, &authority);
    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 100_000_000_000).unwrap();

    // invalid geo (> GEO_MAX = 2^30-1)
    assert_failed_with(
        send(
            &mut svm,
            register_ix(&op.pubkey(), &collection, &tree, 0, 1 << 30, WIREGUARD, 50),
            &op,
        ),
        "InvalidGeo",
    );
    // invalid capabilities (unknown bit)
    assert_failed_with(
        send(
            &mut svm,
            register_ix(&op.pubkey(), &collection, &tree, 0, 100, 1 << 31, 50),
            &op,
        ),
        "InvalidCapabilities",
    );
    // invalid availability (> 100)
    assert_failed_with(
        send(
            &mut svm,
            register_ix(&op.pubkey(), &collection, &tree, 0, 100, WIREGUARD, 101),
            &op,
        ),
        "InvalidAvailability",
    );
}

#[test]
fn register_rejects_when_paused() {
    let (mut svm, authority) = setup();
    let (collection, tree) = setup_with_tree(&mut svm, &authority);
    // pause
    let data = node_registry::instruction::SetPaused { paused: true }.data();
    let metas = node_registry::accounts::AdminRegistry {
        authority: authority.pubkey(),
        registry: registry_pda(),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(node_registry::ID, &data, metas),
        &authority,
    )
    .unwrap();

    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 100_000_000_000).unwrap();
    assert_failed_with(
        send(
            &mut svm,
            register_ix(&op.pubkey(), &collection, &tree, 0, 100, WIREGUARD, 50),
            &op,
        ),
        "Paused",
    );
}

fn update_ix(
    operator: &Pubkey,
    node_id: u64,
    availability: Option<u8>,
    geo: Option<u32>,
) -> Instruction {
    let data = node_registry::instruction::Update {
        geo,
        capabilities: None,
        endpoint_hash: None,
        availability,
    }
    .data();
    let metas = node_registry::accounts::UpdateNode {
        operator: *operator,
        node: node_pda(operator, node_id),
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(node_registry::ID, &data, metas)
}

#[test]
fn update_mutates_active_node() {
    let (mut svm, _authority) = setup();
    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 100_000_000_000).unwrap();
    let pda = set_node_state(&mut svm, &op.pubkey(), 7, node_registry::STATUS_ACTIVE);

    send(
        &mut svm,
        update_ix(&op.pubkey(), 7, Some(80), Some(2048)),
        &op,
    )
    .unwrap();
    let n = get_node(&svm, &pda);
    assert_eq!(n.availability, 80);
    assert_eq!(n.geo, 2048);
}

#[test]
fn update_rejects_non_operator_and_inactive() {
    let (mut svm, _authority) = setup();
    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 100_000_000_000).unwrap();
    set_node_state(&mut svm, &op.pubkey(), 7, node_registry::STATUS_ACTIVE);

    // attacker can't update (PDA seeds use their key → different/absent PDA)
    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), 100_000_000_000).unwrap();
    assert!(send(
        &mut svm,
        update_ix(&attacker.pubkey(), 7, Some(80), None),
        &attacker
    )
    .is_err());

    // inactive node rejects update
    set_node_state(
        &mut svm,
        &op.pubkey(),
        9,
        node_registry::STATUS_DEREGISTERED,
    );
    assert_failed_with(
        send(&mut svm, update_ix(&op.pubkey(), 9, Some(80), None), &op),
        "NodeNotActive",
    );
}

fn deregister_ix(operator: &Pubkey, node_id: u64) -> Instruction {
    let data = node_registry::instruction::Deregister {}.data();
    let metas = node_registry::accounts::Deregister {
        operator: *operator,
        registry: registry_pda(),
        node: node_pda(operator, node_id),
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(node_registry::ID, &data, metas)
}

#[test]
fn deregister_closes_node_state() {
    let (mut svm, authority) = setup();
    init_registry(&mut svm, &authority);
    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 100_000_000_000).unwrap();
    let pda = set_node_state(&mut svm, &op.pubkey(), 7, node_registry::STATUS_ACTIVE);

    // non-operator cannot deregister (seeds derive a different/empty PDA)
    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), 100_000_000_000).unwrap();
    assert!(send(&mut svm, deregister_ix(&attacker.pubkey(), 7), &attacker).is_err());

    // operator closes the NodeState (rent reclaimed)
    send(&mut svm, deregister_ix(&op.pubkey(), 7), &op).unwrap();
    assert!(svm.get_account(&pda).is_none_or(|a| a.lamports == 0));
}

fn set_metrics_authorities_ix(authority: &Pubkey, rep: &Pubkey, stk: &Pubkey) -> Instruction {
    let data = node_registry::instruction::SetMetricsAuthorities {
        reputation_authority: *rep,
        staking_authority: *stk,
    }
    .data();
    let metas = node_registry::accounts::AdminRegistry {
        authority: *authority,
        registry: registry_pda(),
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(node_registry::ID, &data, metas)
}

fn set_stake_ix(staking_authority: &Pubkey, node: &Pubkey, amount: u64) -> Instruction {
    let data = node_registry::instruction::SetStake { amount }.data();
    let metas = node_registry::accounts::SetStake {
        staking_authority: *staking_authority,
        registry: registry_pda(),
        node: *node,
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(node_registry::ID, &data, metas)
}

fn set_reputation_ix(reputation_authority: &Pubkey, node: &Pubkey, bps: u16) -> Instruction {
    let data = node_registry::instruction::SetReputation {
        reputation_bps: bps,
    }
    .data();
    let metas = node_registry::accounts::SetReputation {
        reputation_authority: *reputation_authority,
        registry: registry_pda(),
        node: *node,
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(node_registry::ID, &data, metas)
}

#[test]
fn gated_mirror_writes_node_metrics() {
    let (mut svm, authority) = setup();
    init_registry(&mut svm, &authority);
    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 100_000_000_000).unwrap();
    let node = set_node_state(&mut svm, &op.pubkey(), 7, node_registry::STATUS_ACTIVE);

    let staking_auth = Keypair::new();
    let rep_auth = Keypair::new();
    for k in [&staking_auth, &rep_auth] {
        svm.airdrop(&k.pubkey(), 100_000_000_000).unwrap();
    }

    // admin points the gated writers at the two authorities
    send(
        &mut svm,
        set_metrics_authorities_ix(
            &authority.pubkey(),
            &rep_auth.pubkey(),
            &staking_auth.pubkey(),
        ),
        &authority,
    )
    .unwrap();

    // staking authority mirrors stake; reputation authority cannot
    send(
        &mut svm,
        set_stake_ix(&staking_auth.pubkey(), &node, 12_345),
        &staking_auth,
    )
    .unwrap();
    assert_eq!(get_node(&svm, &node).stake_amount, 12_345);
    assert_failed_with(
        send(
            &mut svm,
            set_stake_ix(&rep_auth.pubkey(), &node, 1),
            &rep_auth,
        ),
        "Unauthorized",
    );

    // reputation authority mirrors reputation; staking authority cannot
    send(
        &mut svm,
        set_reputation_ix(&rep_auth.pubkey(), &node, 15_000),
        &rep_auth,
    )
    .unwrap();
    assert_eq!(get_node(&svm, &node).reputation, 15_000);
    assert_failed_with(
        send(
            &mut svm,
            set_reputation_ix(&staking_auth.pubkey(), &node, 1),
            &staking_auth,
        ),
        "Unauthorized",
    );
}
