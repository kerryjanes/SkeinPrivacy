//! Multi-program LiteSVM tests for the DAO: load governance + staking +
//! rewards-settlement, drive the full proposal lifecycle (create → add tx →
//! activate → stake-weighted vote → finalize → timelock → execute), and prove a
//! passed proposal (a) edits the governed `ProtocolConfig` which `pay_traffic`
//! then enforces, and (b) rotates another program's authority via the DAO PDA.
//! Also covers the double-vote and unlocked-position guards and the quorum path.

#![allow(clippy::result_large_err)]

use {
    anchor_lang::{
        solana_program::instruction::{AccountMeta, Instruction},
        AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas,
    },
    governance::{ProtocolParams, VoteKind},
    litesvm::{types::TransactionResult, LiteSVM},
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

const GOVERNANCE_BYTES: &[u8] = include_bytes!("../../../target/deploy/governance.so");
const STAKING_BYTES: &[u8] = include_bytes!("../../../target/deploy/staking.so");
const SETTLEMENT_BYTES: &[u8] = include_bytes!("../../../target/deploy/rewards_settlement.so");
const DECIMALS: u8 = 9;

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(governance::ID, GOVERNANCE_BYTES).unwrap();
    svm.add_program(staking::ID, STAKING_BYTES).unwrap();
    svm.add_program(rewards_settlement::ID, SETTLEMENT_BYTES)
        .unwrap();
    let admin = Keypair::new();
    svm.airdrop(&admin.pubkey(), 1_000_000_000_000).unwrap();
    (svm, admin)
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

// ---- PDAs ----
fn gov_config_pda() -> Pubkey {
    Pubkey::find_program_address(&[governance::GOVERNANCE_CONFIG_SEED], &governance::ID).0
}
fn gov_authority_pda() -> Pubkey {
    Pubkey::find_program_address(&[governance::GOVERNANCE_AUTHORITY_SEED], &governance::ID).0
}
fn protocol_config_pda() -> Pubkey {
    Pubkey::find_program_address(&[governance::PROTOCOL_CONFIG_SEED], &governance::ID).0
}
fn proposal_pda(id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[governance::PROPOSAL_SEED, &id.to_le_bytes()],
        &governance::ID,
    )
    .0
}
fn proposal_tx_pda(proposal: &Pubkey, index: u16) -> Pubkey {
    Pubkey::find_program_address(
        &[
            governance::PROPOSAL_TX_SEED,
            proposal.as_ref(),
            &index.to_le_bytes(),
        ],
        &governance::ID,
    )
    .0
}
fn vote_pda(proposal: &Pubkey, position: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[governance::VOTE_SEED, proposal.as_ref(), position.as_ref()],
        &governance::ID,
    )
    .0
}
fn position_pda(op: &Pubkey, node_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[staking::STAKE_SEED, op.as_ref(), &node_id.to_le_bytes()],
        &staking::ID,
    )
    .0
}
fn staking_config_pda() -> Pubkey {
    Pubkey::find_program_address(&[staking::CONFIG_SEED], &staking::ID).0
}
fn distributor_pda() -> Pubkey {
    Pubkey::find_program_address(
        &[rewards_settlement::DISTRIBUTOR_SEED],
        &rewards_settlement::ID,
    )
    .0
}

// ---- account crafting ----
fn craft_position(
    svm: &mut LiteSVM,
    operator: &Pubkey,
    node_id: u64,
    amount: u64,
    locked_until: i64,
) -> Pubkey {
    let (pda, bump) = Pubkey::find_program_address(
        &[
            staking::STAKE_SEED,
            operator.as_ref(),
            &node_id.to_le_bytes(),
        ],
        &staking::ID,
    );
    let pos = staking::StakePosition {
        operator: *operator,
        node_id,
        mint: Pubkey::new_unique(),
        vault: Pubkey::new_unique(),
        amount,
        unbonding_amount: 0,
        locked_until,
        unbonding_until: 0,
        bump,
    };
    let mut data = Vec::new();
    pos.try_serialize(&mut data).unwrap();
    svm.set_account(
        pda,
        Account {
            lamports: 10_000_000,
            data,
            owner: staking::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
    pda
}

fn craft_staking_config(svm: &mut LiteSVM, authority: &Pubkey, mint: &Pubkey, treasury: &Pubkey) {
    let (pda, bump) = Pubkey::find_program_address(&[staking::CONFIG_SEED], &staking::ID);
    let cfg = staking::StakingConfig {
        authority: *authority,
        slash_authority: Pubkey::new_unique(),
        treasury: *treasury,
        mint: *mint,
        unbonding_seconds: 60,
        bump,
    };
    let mut data = Vec::new();
    cfg.try_serialize(&mut data).unwrap();
    svm.set_account(
        pda,
        Account {
            lamports: 10_000_000,
            data,
            owner: staking::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

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
            owner: spl_token_interface::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
    mint
}
fn create_token_account(svm: &mut LiteSVM, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Pubkey {
    let addr = Pubkey::new_unique();
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
            owner: spl_token_interface::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
    addr
}
fn token_amount(svm: &LiteSVM, addr: &Pubkey) -> u64 {
    TokenState::unpack(&svm.get_account(addr).unwrap().data)
        .unwrap()
        .amount
}
fn mint_supply(svm: &LiteSVM, mint: &Pubkey) -> u64 {
    MintState::unpack(&svm.get_account(mint).unwrap().data)
        .unwrap()
        .supply
}
fn set_mint_supply(svm: &mut LiteSVM, mint: &Pubkey, supply: u64) {
    let mut s = MintState::unpack(&svm.get_account(mint).unwrap().data).unwrap();
    s.supply = supply;
    let mut data = vec![0u8; MintState::LEN];
    MintState::pack(s, &mut data).unwrap();
    let mut acc = svm.get_account(mint).unwrap();
    acc.data = data;
    svm.set_account(*mint, acc).unwrap();
}

fn craft_distributor(svm: &mut LiteSVM, mint: &Pubkey, vault: &Pubkey, treasury: &Pubkey) {
    let (pda, bump) = Pubkey::find_program_address(
        &[rewards_settlement::DISTRIBUTOR_SEED],
        &rewards_settlement::ID,
    );
    let d = rewards_settlement::Distributor {
        authority: Pubkey::new_unique(),
        poster_authority: Pubkey::new_unique(),
        dispute_authority: Pubkey::new_unique(),
        reward_mint: *mint,
        reward_vault: *vault,
        treasury: *treasury,
        dispute_window_seconds: 0,
        clawback_window_seconds: 0,
        current_epoch: 0,
        cumulative_obligated: 0,
        cumulative_claimed: 0,
        bump,
    };
    let mut data = Vec::new();
    d.try_serialize(&mut data).unwrap();
    svm.set_account(
        pda,
        Account {
            lamports: 10_000_000,
            data,
            owner: rewards_settlement::ID,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

// ---- governance ix builders ----
#[allow(clippy::too_many_arguments)]
fn init_governance(
    svm: &mut LiteSVM,
    admin: &Keypair,
    quorum: u64,
    threshold_bps: u32,
    voting: i64,
    delay: i64,
    min_stake: u64,
) {
    let data = governance::instruction::InitializeGovernance {
        default_quorum: quorum,
        default_approval_threshold_bps: threshold_bps,
        voting_period_seconds: voting,
        execution_delay_seconds: delay,
        min_proposal_stake: min_stake,
    }
    .data();
    let metas = governance::accounts::InitializeGovernance {
        authority: admin.pubkey(),
        gov_mint: Pubkey::new_unique(),
        governance_config: gov_config_pda(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(governance::ID, &data, metas),
        admin,
        &[admin],
    )
    .unwrap();
}

fn init_protocol_config(svm: &mut LiteSVM, admin: &Keypair, config_authority: Pubkey) {
    let data = governance::instruction::InitializeProtocolConfig {
        config_authority,
        dispute_window_seconds: 300,
        clawback_window_seconds: 600,
    }
    .data();
    let metas = governance::accounts::InitializeProtocolConfig {
        authority: admin.pubkey(),
        governance_config: gov_config_pda(),
        protocol_config: protocol_config_pda(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(governance::ID, &data, metas),
        admin,
        &[admin],
    )
    .unwrap();
}

fn create_proposal(
    svm: &mut LiteSVM,
    proposer: &Keypair,
    node_id: u64,
    id: u64,
    name: &str,
) -> TransactionResult {
    let data = governance::instruction::CreateProposal {
        node_id,
        name: name.to_string(),
    }
    .data();
    let metas = governance::accounts::CreateProposal {
        proposer: proposer.pubkey(),
        governance_config: gov_config_pda(),
        position: position_pda(&proposer.pubkey(), node_id),
        proposal: proposal_pda(id),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(governance::ID, &data, metas),
        proposer,
        &[proposer],
    )
}

fn add_transaction(
    svm: &mut LiteSVM,
    proposer: &Keypair,
    id: u64,
    index: u16,
    program_id: Pubkey,
    inner: &[AccountMeta],
    inner_data: Vec<u8>,
) {
    let accounts: Vec<governance::TxAccountMeta> = inner
        .iter()
        .map(|m| governance::TxAccountMeta {
            pubkey: m.pubkey,
            is_signer: m.is_signer,
            is_writable: m.is_writable,
        })
        .collect();
    let data = governance::instruction::AddTransaction {
        index,
        program_id,
        accounts,
        data: inner_data,
    }
    .data();
    let metas = governance::accounts::AddTransaction {
        proposer: proposer.pubkey(),
        proposal: proposal_pda(id),
        proposal_transaction: proposal_tx_pda(&proposal_pda(id), index),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(governance::ID, &data, metas),
        proposer,
        &[proposer],
    )
    .unwrap();
}

fn activate(svm: &mut LiteSVM, proposer: &Keypair, id: u64) {
    let data = governance::instruction::ActivateProposal {}.data();
    let metas = governance::accounts::ActivateProposal {
        proposer: proposer.pubkey(),
        governance_config: gov_config_pda(),
        proposal: proposal_pda(id),
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(governance::ID, &data, metas),
        proposer,
        &[proposer],
    )
    .unwrap();
}

fn cast_vote(
    svm: &mut LiteSVM,
    voter: &Keypair,
    id: u64,
    node_id: u64,
    vote: VoteKind,
) -> TransactionResult {
    let data = governance::instruction::CastVote { node_id, vote }.data();
    let pos = position_pda(&voter.pubkey(), node_id);
    let metas = governance::accounts::CastVote {
        voter: voter.pubkey(),
        proposal: proposal_pda(id),
        position: pos,
        vote_record: vote_pda(&proposal_pda(id), &pos),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(governance::ID, &data, metas),
        voter,
        &[voter],
    )
}

fn finalize(svm: &mut LiteSVM, who: &Keypair, id: u64) {
    let data = governance::instruction::FinalizeProposal {}.data();
    let metas = governance::accounts::FinalizeProposal {
        finalizer: who.pubkey(),
        governance_config: gov_config_pda(),
        proposal: proposal_pda(id),
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(governance::ID, &data, metas),
        who,
        &[who],
    )
    .unwrap();
}

fn execute(
    svm: &mut LiteSVM,
    who: &Keypair,
    id: u64,
    index: u16,
    inner_program: Pubkey,
    inner: &[AccountMeta],
) -> TransactionResult {
    let data = governance::instruction::ExecuteTransaction {}.data();
    let mut metas = governance::accounts::ExecuteTransaction {
        executor: who.pubkey(),
        proposal: proposal_pda(id),
        proposal_transaction: proposal_tx_pda(&proposal_pda(id), index),
        governance_authority: gov_authority_pda(),
    }
    .to_account_metas(None);
    // remaining: the inner program + its accounts (DAO PDA never a real signer here).
    metas.push(AccountMeta::new_readonly(inner_program, false));
    for m in inner {
        if m.is_writable {
            metas.push(AccountMeta::new(m.pubkey, false));
        } else {
            metas.push(AccountMeta::new_readonly(m.pubkey, false));
        }
    }
    send(
        svm,
        Instruction::new_with_bytes(governance::ID, &data, metas),
        who,
        &[who],
    )
}

fn proposal_state(svm: &LiteSVM, id: u64) -> governance::ProposalState {
    governance::Proposal::try_deserialize(
        &mut svm.get_account(&proposal_pda(id)).unwrap().data.as_slice(),
    )
    .unwrap()
    .state
}
fn protocol_config(svm: &LiteSVM) -> governance::ProtocolConfig {
    governance::ProtocolConfig::try_deserialize(
        &mut svm
            .get_account(&protocol_config_pda())
            .unwrap()
            .data
            .as_slice(),
    )
    .unwrap()
}

// The inner instruction that updates the protocol split to 60/30/10.
fn update_split_inner() -> (Pubkey, Vec<AccountMeta>, Vec<u8>) {
    let params = ProtocolParams {
        split_nodes_bps: 6_000,
        split_burn_bps: 3_000,
        split_treasury_bps: 1_000,
        dispute_window_seconds: 300,
        clawback_window_seconds: 600,
        base_rate_per_gb: 1_000_000_000_000,
        geo_bonus_max_bps: 5_000,
        reputation_min_bps: 5_000,
        reputation_max_bps: 20_000,
        staking_bonus_bps: 2_000,
        staking_bonus_threshold: 10_000_000_000_000,
        bootstrap_node_limit: 10_000,
        bootstrap_bonus_bps: 5_000,
        bootstrap_end_ts: 0,
    };
    let data = governance::instruction::UpdateProtocolConfig { params }.data();
    let inner = vec![
        AccountMeta::new_readonly(gov_authority_pda(), true), // authority = DAO PDA (signer via seeds)
        AccountMeta::new(protocol_config_pda(), false),
    ];
    (governance::ID, inner, data)
}

const FUTURE: i64 = 1_000_000;

#[test]
fn proposal_passes_updates_protocol_config_and_pay_traffic_follows() {
    let (mut svm, admin) = setup();
    set_clock(&mut svm, 1_000);
    // governance: quorum 10k, 60% threshold, 100s vote, 50s timelock, min propose 1k.
    init_governance(&mut svm, &admin, 10_000, 6_000, 100, 50, 1_000);
    // ProtocolConfig authority = DAO PDA, so only a passed proposal can edit it.
    init_protocol_config(&mut svm, &admin, gov_authority_pda());
    assert_eq!(protocol_config(&svm).split_nodes_bps, 7_000);

    let a = Keypair::new();
    svm.airdrop(&a.pubkey(), 1_000_000_000).unwrap();
    craft_position(&mut svm, &a.pubkey(), 1, 30_000, FUTURE);

    create_proposal(&mut svm, &a, 1, 0, "lower node share").unwrap();
    let (prog, inner, idata) = update_split_inner();
    add_transaction(&mut svm, &a, 0, 0, prog, &inner, idata);
    activate(&mut svm, &a, 0);

    // 30k yes meets quorum (10k) and threshold (100% >= 60%).
    cast_vote(&mut svm, &a, 0, 1, VoteKind::Yes).unwrap();

    // can't execute before finalize / timelock
    assert!(execute(&mut svm, &a, 0, 0, prog, &inner).is_err());

    set_clock(&mut svm, 1_200); // past voting_ends_at (1000+100)
    finalize(&mut svm, &a, 0);
    assert_eq!(
        proposal_state(&svm, 0),
        governance::ProposalState::Succeeded
    );

    // timelock: execution_eta = 1200 + 50
    assert_failed_with(
        execute(&mut svm, &a, 0, 0, prog, &inner),
        "TimelockNotElapsed",
    );
    set_clock(&mut svm, 1_300);
    execute(&mut svm, &a, 0, 0, prog, &inner).unwrap();

    let pc = protocol_config(&svm);
    assert_eq!(pc.split_nodes_bps, 6_000);
    assert_eq!(pc.split_burn_bps, 3_000);
    assert_eq!(proposal_state(&svm, 0), governance::ProposalState::Executed);

    // pay_traffic now enforces the DAO-set 60/30/10 split.
    let mint = create_mint(&mut svm);
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000).unwrap();
    let amount = 1_000_000u64;
    let payer_ta = create_token_account(&mut svm, &mint, &payer.pubkey(), amount);
    let vault = create_token_account(&mut svm, &mint, &distributor_pda(), 0);
    let treasury = create_token_account(&mut svm, &mint, &Pubkey::new_unique(), 0);
    set_mint_supply(&mut svm, &mint, amount);
    craft_distributor(&mut svm, &mint, &vault, &treasury);

    let data = rewards_settlement::instruction::PayTraffic { amount }.data();
    let metas = rewards_settlement::accounts::PayTraffic {
        payer: payer.pubkey(),
        distributor: distributor_pda(),
        protocol_config: protocol_config_pda(),
        reward_mint: mint,
        payer_token_account: payer_ta,
        reward_vault: vault,
        treasury,
        token_program: spl_token_interface::ID,
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        &payer,
        &[&payer],
    )
    .unwrap();
    assert_eq!(token_amount(&svm, &vault), 600_000); // 60%
    assert_eq!(token_amount(&svm, &treasury), 100_000); // 10%
    assert_eq!(mint_supply(&svm, &mint), amount - 300_000); // 30% burned
}

#[test]
fn double_vote_and_unlocked_position_are_rejected() {
    let (mut svm, admin) = setup();
    set_clock(&mut svm, 1_000);
    init_governance(&mut svm, &admin, 10_000, 6_000, 100, 50, 1_000);
    init_protocol_config(&mut svm, &admin, gov_authority_pda());

    let a = Keypair::new();
    svm.airdrop(&a.pubkey(), 1_000_000_000).unwrap();
    craft_position(&mut svm, &a.pubkey(), 1, 30_000, FUTURE);
    create_proposal(&mut svm, &a, 1, 0, "p").unwrap();
    activate(&mut svm, &a, 0);

    cast_vote(&mut svm, &a, 0, 1, VoteKind::Yes).unwrap();
    // same position again → the VoteRecord PDA already exists
    assert!(cast_vote(&mut svm, &a, 0, 1, VoteKind::Yes).is_err());

    // a position that unlocks before voting ends cannot vote
    let b = Keypair::new();
    svm.airdrop(&b.pubkey(), 1_000_000_000).unwrap();
    craft_position(&mut svm, &b.pubkey(), 2, 5_000, 1_050); // < voting_ends_at 1100
    assert_failed_with(
        cast_vote(&mut svm, &b, 0, 2, VoteKind::Yes),
        "PositionUnlockedBeforeVoteEnds",
    );
}

#[test]
fn quorum_not_met_defeats_proposal() {
    let (mut svm, admin) = setup();
    set_clock(&mut svm, 1_000);
    init_governance(&mut svm, &admin, 10_000, 6_000, 100, 50, 1_000);
    init_protocol_config(&mut svm, &admin, gov_authority_pda());

    let a = Keypair::new();
    svm.airdrop(&a.pubkey(), 1_000_000_000).unwrap();
    craft_position(&mut svm, &a.pubkey(), 1, 30_000, FUTURE); // proposer stake
    let b = Keypair::new();
    svm.airdrop(&b.pubkey(), 1_000_000_000).unwrap();
    craft_position(&mut svm, &b.pubkey(), 2, 5_000, FUTURE); // only 5k votes (< 10k quorum)

    create_proposal(&mut svm, &a, 1, 0, "p").unwrap();
    activate(&mut svm, &a, 0);
    cast_vote(&mut svm, &b, 0, 2, VoteKind::Yes).unwrap();

    set_clock(&mut svm, 1_200);
    finalize(&mut svm, &a, 0);
    assert_eq!(proposal_state(&svm, 0), governance::ProposalState::Defeated);
}

#[test]
fn executes_cross_program_authority_rotation() {
    let (mut svm, admin) = setup();
    set_clock(&mut svm, 1_000);
    init_governance(&mut svm, &admin, 10_000, 6_000, 100, 50, 1_000);
    init_protocol_config(&mut svm, &admin, gov_authority_pda());

    // staking config whose authority is the DAO PDA, so the DAO can rotate it.
    let mint = create_mint(&mut svm);
    let treasury = create_token_account(&mut svm, &mint, &Pubkey::new_unique(), 0);
    craft_staking_config(&mut svm, &gov_authority_pda(), &mint, &treasury);

    let a = Keypair::new();
    svm.airdrop(&a.pubkey(), 1_000_000_000).unwrap();
    craft_position(&mut svm, &a.pubkey(), 1, 30_000, FUTURE);

    create_proposal(&mut svm, &a, 1, 0, "rotate staking authority").unwrap();
    let new_authority = Pubkey::new_unique();
    let inner_data = staking::instruction::TransferAuthority { new_authority }.data();
    let inner = vec![
        AccountMeta::new_readonly(gov_authority_pda(), true), // config.authority signer = DAO PDA
        AccountMeta::new(staking_config_pda(), false),
    ];
    add_transaction(&mut svm, &a, 0, 0, staking::ID, &inner, inner_data);
    activate(&mut svm, &a, 0);
    cast_vote(&mut svm, &a, 0, 1, VoteKind::Yes).unwrap();

    set_clock(&mut svm, 1_200);
    finalize(&mut svm, &a, 0);
    set_clock(&mut svm, 1_300);
    execute(&mut svm, &a, 0, 0, staking::ID, &inner).unwrap();

    let cfg = staking::StakingConfig::try_deserialize(
        &mut svm
            .get_account(&staking_config_pda())
            .unwrap()
            .data
            .as_slice(),
    )
    .unwrap();
    assert_eq!(cfg.authority, new_authority);
}
