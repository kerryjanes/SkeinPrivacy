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
