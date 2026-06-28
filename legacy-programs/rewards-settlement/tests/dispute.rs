//! Multi-program LiteSVM test for the dispute path: load node-registry +
//! staking + reputation + rewards-settlement, stake + score a real node
//! (mirrored into `NodeState`), re-point `staking.slash_authority` /
//! `reputation.oracle` at the settlement `[b"authority"]` PDA, then `dispute`
//! a fraudulent epoch leaf and prove the settlement program — signing only as
//! its program PDA — slashes the stake, penalizes the reputation, mirrors both
//! reductions into `NodeState`, and permanently blocks the payout.

#![allow(clippy::result_large_err)]

use {
    anchor_lang::{
        solana_program::instruction::Instruction, AccountDeserialize, AccountSerialize,
        InstructionData, ToAccountMetas,
    },
    litesvm::{types::TransactionResult, LiteSVM},
    weft_primitives::merkle::{hash_reward_leaf, merkle_root},
    solana_account::Account,
    solana_clock::Clock,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_program_option::COption,
    solana_program_pack::Pack,
    solana_pubkey::Pubkey,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
    spl_token_interface::state::{Account as TokenState, AccountState, Mint as MintState},
};

const NODE_REGISTRY_BYTES: &[u8] = include_bytes!("../../../target/deploy/node_registry.so");
const STAKING_BYTES: &[u8] = include_bytes!("../../../target/deploy/staking.so");
const REPUTATION_BYTES: &[u8] = include_bytes!("../../../target/deploy/reputation.so");
const SETTLEMENT_BYTES: &[u8] = include_bytes!("../../../target/deploy/rewards_settlement.so");
const DECIMALS: u8 = 9;

fn token_program_id() -> Pubkey {
    spl_token_interface::ID
}

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(node_registry::ID, NODE_REGISTRY_BYTES)
        .unwrap();
    svm.add_program(staking::ID, STAKING_BYTES).unwrap();
    svm.add_program(reputation::ID, REPUTATION_BYTES).unwrap();
    svm.add_program(rewards_settlement::ID, SETTLEMENT_BYTES)
        .unwrap();
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

// ---- PDAs ----
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
fn staking_config_pda() -> Pubkey {
    Pubkey::find_program_address(&[staking::CONFIG_SEED], &staking::ID).0
}
fn position_pda(op: &Pubkey, id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[staking::STAKE_SEED, op.as_ref(), &id.to_le_bytes()],
        &staking::ID,
    )
    .0
}
fn stake_vault_pda(position: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[staking::VAULT_SEED, position.as_ref()], &staking::ID).0
}
fn staking_authority_pda() -> Pubkey {
    Pubkey::find_program_address(&[staking::AUTHORITY_SEED], &staking::ID).0
}
fn reputation_config_pda() -> Pubkey {
    Pubkey::find_program_address(&[reputation::CONFIG_SEED], &reputation::ID).0
}
fn reputation_state_pda(op: &Pubkey, id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[reputation::STATE_SEED, op.as_ref(), &id.to_le_bytes()],
        &reputation::ID,
    )
    .0
}
fn reputation_authority_pda() -> Pubkey {
    Pubkey::find_program_address(&[reputation::AUTHORITY_SEED], &reputation::ID).0
}
fn distributor_pda() -> Pubkey {
    Pubkey::find_program_address(
        &[rewards_settlement::DISTRIBUTOR_SEED],
        &rewards_settlement::ID,
    )
    .0
}
fn settlement_vault_pda() -> Pubkey {
    Pubkey::find_program_address(&[rewards_settlement::VAULT_SEED], &rewards_settlement::ID).0
}
fn epoch_pda(epoch: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[rewards_settlement::EPOCH_SEED, &epoch.to_le_bytes()],
        &rewards_settlement::ID,
    )
    .0
}
fn claim_pda(epoch: u64, op: &Pubkey, id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            rewards_settlement::CLAIM_SEED,
            &epoch.to_le_bytes(),
            op.as_ref(),
            &id.to_le_bytes(),
        ],
        &rewards_settlement::ID,
    )
    .0
}
fn settlement_authority_pda() -> Pubkey {
    Pubkey::find_program_address(
        &[rewards_settlement::AUTHORITY_SEED],
        &rewards_settlement::ID,
    )
    .0
}

// ---- token helpers ----
fn create_mint(svm: &mut LiteSVM) -> Pubkey {
    let mint = Pubkey::new_unique();
    let s = MintState {
        mint_authority: COption::Some(Pubkey::new_unique()),
        supply: 0,
        decimals: DECIMALS,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; MintState::LEN];
    MintState::pack(s, &mut data).unwrap();
    svm.set_account(
        mint,
        Account {
            lamports: 1_000_000_000,
            data,
            owner: token_program_id(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
    mint
}
fn write_token_account(
    svm: &mut LiteSVM,
    addr: Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    amount: u64,
) {
    let s = TokenState {
        mint: *mint,
        owner: *owner,
        amount,
        delegate: COption::None,
        state: AccountState::Initialized,
        is_native: COption::None,
        delegated_amount: 0,
        close_authority: COption::None,
    };
    let mut data = vec![0u8; TokenState::LEN];
    TokenState::pack(s, &mut data).unwrap();
    svm.set_account(
        addr,
        Account {
            lamports: 1_000_000_000,
            data,
            owner: token_program_id(),
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}
fn create_token_account(svm: &mut LiteSVM, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Pubkey {
    let addr = Pubkey::new_unique();
    write_token_account(svm, addr, mint, owner, amount);
    addr
}
fn token_amount(svm: &LiteSVM, addr: &Pubkey) -> u64 {
    TokenState::unpack(&svm.get_account(addr).unwrap().data)
        .unwrap()
        .amount
}

// ---- node-registry setup ----
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
        reputation_authority: reputation_authority_pda(),
        staking_authority: staking_authority_pda(),
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
        sequence: 0,
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
fn node_state(svm: &LiteSVM, node: &Pubkey) -> node_registry::NodeState {
    node_registry::NodeState::try_deserialize(&mut svm.get_account(node).unwrap().data.as_slice())
        .unwrap()
}

#[test]
fn dispute_slashes_penalizes_and_blocks_payout() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, 1_000);
    init_registry(&mut svm, &authority);

    let mint = create_mint(&mut svm);
    let stake_treasury = create_token_account(&mut svm, &mint, &Pubkey::new_unique(), 0);

    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 1_000_000_000_000).unwrap();
    let node_id = 1u64;
    let node = craft_node(&mut svm, &op.pubkey(), node_id);

    // --- staking config + stake (mirrors into NodeState) ---
    let data = staking::instruction::InitializeConfig {
        unbonding_seconds: 100,
    }
    .data();
    let metas = staking::accounts::InitializeConfig {
        authority: authority.pubkey(),
        mint,
        treasury: stake_treasury,
        slash_authority: authority.pubkey(),
        config: staking_config_pda(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(staking::ID, &data, metas),
        &authority,
        &[&authority],
    )
    .unwrap();

    let op_ta = create_token_account(&mut svm, &mint, &op.pubkey(), 50_000);
    let position = position_pda(&op.pubkey(), node_id);
    let data = staking::instruction::Stake {
        node_id,
        amount: 30_000,
        lock_duration: 0,
    }
    .data();
    let metas = staking::accounts::Stake {
        operator: op.pubkey(),
        config: staking_config_pda(),
        position,
        vault: stake_vault_pda(&position),
        operator_token_account: op_ta,
        mint,
        program_authority: staking_authority_pda(),
        node_registry_program: node_registry::ID,
        registry: registry_pda(),
        node: Some(node),
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(staking::ID, &data, metas),
        &op,
        &[&op],
    )
    .unwrap();
    assert_eq!(node_state(&svm, &node).stake_amount, 30_000);

    // --- reputation config + score (mirrors into NodeState) ---
    let data = reputation::instruction::InitializeConfig {}.data();
    let metas = reputation::accounts::InitializeConfig {
        authority: authority.pubkey(),
        oracle: authority.pubkey(),
        config: reputation_config_pda(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(reputation::ID, &data, metas),
        &authority,
        &[&authority],
    )
    .unwrap();

    let data = reputation::instruction::UpdateMetrics {
        node_id,
        uptime_bps: 9_000,
        speed_bps: 9_000,
        review_bps: 9_000,
    }
    .data();
    let metas = reputation::accounts::UpdateMetrics {
        oracle: authority.pubkey(),
        config: reputation_config_pda(),
        operator: op.pubkey(),
        state: reputation_state_pda(&op.pubkey(), node_id),
        program_authority: reputation_authority_pda(),
        node_registry_program: node_registry::ID,
        registry: registry_pda(),
        node: Some(node),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(reputation::ID, &data, metas),
        &authority,
        &[&authority],
    )
    .unwrap();
    let rep_before = node_state(&svm, &node).reputation;
    assert!(rep_before > 0, "reputation should be seeded");

    // --- re-point slash_authority + oracle at the settlement PDA (chunk-2 admin ixs) ---
    let sa = settlement_authority_pda();
    let data = staking::instruction::SetSlashAuthority {
        new_slash_authority: sa,
    }
    .data();
    let metas = staking::accounts::SetSlashAuthority {
        authority: authority.pubkey(),
        config: staking_config_pda(),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(staking::ID, &data, metas),
        &authority,
        &[&authority],
    )
    .unwrap();

    let data = reputation::instruction::SetOracle { new_oracle: sa }.data();
    let metas = reputation::accounts::SetOracle {
        authority: authority.pubkey(),
        config: reputation_config_pda(),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(reputation::ID, &data, metas),
        &authority,
        &[&authority],
    )
    .unwrap();

    // --- settlement distributor + a posted epoch with a (fraudulent) leaf ---
    let dispute_auth = Keypair::new();
    svm.airdrop(&dispute_auth.pubkey(), 1_000_000_000).unwrap();
    let poster = Keypair::new();
    svm.airdrop(&poster.pubkey(), 1_000_000_000).unwrap();
    let reward_treasury = create_token_account(&mut svm, &mint, &Pubkey::new_unique(), 0);

    let data = rewards_settlement::instruction::InitializeDistributor {
        dispute_window_seconds: 100,
        clawback_window_seconds: 1000,
    }
    .data();
    let metas = rewards_settlement::accounts::InitializeDistributor {
        authority: authority.pubkey(),
        reward_mint: mint,
        poster_authority: poster.pubkey(),
        dispute_authority: dispute_auth.pubkey(),
        treasury: reward_treasury,
        distributor: distributor_pda(),
        reward_vault: settlement_vault_pda(),
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        &authority,
        &[&authority],
    )
    .unwrap();

    // seed the settlement vault so the epoch is solvent
    let reward_amount = 12_345u64;
    write_token_account(
        &mut svm,
        settlement_vault_pda(),
        &mint,
        &distributor_pda(),
        reward_amount,
    );

    let epoch = 1u64;
    let leaves = [hash_reward_leaf(
        epoch,
        &op.pubkey().to_bytes(),
        node_id,
        reward_amount,
    )];
    let root = merkle_root(&leaves);
    let data = rewards_settlement::instruction::PostEpoch {
        epoch,
        merkle_root: root,
        total_reward: reward_amount,
        num_nodes: 1,
    }
    .data();
    let metas = rewards_settlement::accounts::PostEpoch {
        poster: poster.pubkey(),
        distributor: distributor_pda(),
        reward_vault: settlement_vault_pda(),
        epoch_distribution: epoch_pda(epoch),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        &poster,
        &[&poster],
    )
    .unwrap();

    // --- DISPUTE: settlement program slashes + penalizes the node ---
    let stake_before = staking::StakePosition::try_deserialize(
        &mut svm.get_account(&position).unwrap().data.as_slice(),
    )
    .unwrap()
    .amount;
    let treasury_before = token_amount(&svm, &stake_treasury);

    let slash_amount = 10_000u64;
    let severity_bps = 5_000u32;
    let data = rewards_settlement::instruction::Dispute {
        epoch,
        node_id,
        amount: reward_amount,
        severity_bps,
        slash_amount,
    }
    .data();
    let metas = rewards_settlement::accounts::Dispute {
        dispute_authority: dispute_auth.pubkey(),
        distributor: distributor_pda(),
        epoch_distribution: epoch_pda(epoch),
        operator: op.pubkey(),
        claim_status: claim_pda(epoch, &op.pubkey(), node_id),
        program_authority: settlement_authority_pda(),
        staking_program: staking::ID,
        staking_config: staking_config_pda(),
        staking_position: position,
        staking_vault: stake_vault_pda(&position),
        staking_treasury: stake_treasury,
        stake_mint: mint,
        staking_program_authority: staking_authority_pda(),
        reputation_program: reputation::ID,
        reputation_config: reputation_config_pda(),
        reputation_state: reputation_state_pda(&op.pubkey(), node_id),
        reputation_program_authority: reputation_authority_pda(),
        node_registry_program: node_registry::ID,
        registry: registry_pda(),
        node: Some(node),
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        &dispute_auth,
        &[&dispute_auth],
    )
    .unwrap();

    // stake slashed (vault → staking treasury), mirrored into NodeState
    let stake_after = staking::StakePosition::try_deserialize(
        &mut svm.get_account(&position).unwrap().data.as_slice(),
    )
    .unwrap()
    .amount;
    assert_eq!(stake_after, stake_before - slash_amount);
    assert_eq!(node_state(&svm, &node).stake_amount, stake_after);
    assert_eq!(
        token_amount(&svm, &stake_treasury),
        treasury_before + slash_amount
    );

    // reputation penalized (~halved), mirrored into NodeState
    let rep_after = node_state(&svm, &node).reputation;
    assert!(
        rep_after < rep_before,
        "reputation should drop: {rep_before} -> {rep_after}"
    );

    // the disputed ClaimStatus exists and is flagged
    let cs = svm
        .get_account(&claim_pda(epoch, &op.pubkey(), node_id))
        .unwrap();
    let parsed = rewards_settlement::ClaimStatus::try_deserialize(&mut cs.data.as_slice()).unwrap();
    assert!(parsed.disputed);

    // payout is permanently blocked: a claim for this leaf can't init the same PDA
    set_clock(&mut svm, 2_000);
    let op_reward_ta = create_token_account(&mut svm, &mint, &op.pubkey(), 0);
    let data = rewards_settlement::instruction::Claim {
        epoch,
        node_id,
        amount: reward_amount,
        proof: Vec::<[u8; 32]>::new(),
    }
    .data();
    let metas = rewards_settlement::accounts::Claim {
        claimant: op.pubkey(),
        distributor: distributor_pda(),
        epoch_distribution: epoch_pda(epoch),
        operator: op.pubkey(),
        claim_status: claim_pda(epoch, &op.pubkey(), node_id),
        reward_mint: mint,
        reward_vault: settlement_vault_pda(),
        operator_token_account: op_reward_ta,
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    assert!(
        send(
            &mut svm,
            Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
            &op,
            &[&op],
        )
        .is_err(),
        "claim on a disputed leaf must fail (ClaimStatus PDA already exists)"
    );
}
