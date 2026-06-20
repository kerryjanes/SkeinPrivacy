//! LiteSVM tests for the settlement core: the 70/20/10 pay split + burn, the
//! poster-gated solvency-checked epoch post, an on-chain-verified merkle claim
//! (the real `weft-primitives` tree the aggregator will build), double-claim
//! and dispute-window rejection, and the clawback sweep.

#![allow(clippy::result_large_err)]

use {
    anchor_lang::{
        solana_program::instruction::Instruction, AccountDeserialize, InstructionData,
        ToAccountMetas,
    },
    litesvm::{types::TransactionResult, LiteSVM},
    weft_primitives::{
        merkle::{hash_reward_leaf, merkle_proof, merkle_root},
        split_payment,
    },
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

const SETTLEMENT_BYTES: &[u8] = include_bytes!("../../../target/deploy/rewards_settlement.so");
const DECIMALS: u8 = 9;

fn token_program_id() -> Pubkey {
    spl_token_interface::ID
}

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
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

fn assert_failed_with(res: TransactionResult, code: &str) {
    let f = res.expect_err("expected failure");
    let logs = f.meta.logs.join("\n");
    assert!(logs.contains(code), "expected `{code}`, got:\n{logs}");
}

// ---- PDAs ----
fn distributor_pda() -> Pubkey {
    Pubkey::find_program_address(
        &[rewards_settlement::DISTRIBUTOR_SEED],
        &rewards_settlement::ID,
    )
    .0
}
fn vault_pda() -> Pubkey {
    Pubkey::find_program_address(&[rewards_settlement::VAULT_SEED], &rewards_settlement::ID).0
}
fn epoch_pda(epoch: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[rewards_settlement::EPOCH_SEED, &epoch.to_le_bytes()],
        &rewards_settlement::ID,
    )
    .0
}
fn claim_pda(epoch: u64, operator: &Pubkey, node_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            rewards_settlement::CLAIM_SEED,
            &epoch.to_le_bytes(),
            operator.as_ref(),
            &node_id.to_le_bytes(),
        ],
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
fn set_mint_supply(svm: &mut LiteSVM, mint: &Pubkey, supply: u64) {
    let mut data = svm.get_account(mint).unwrap().data;
    let mut s = MintState::unpack(&data).unwrap();
    s.supply = supply;
    MintState::pack(s, &mut data).unwrap();
    let mut acc = svm.get_account(mint).unwrap();
    acc.data = data;
    svm.set_account(*mint, acc).unwrap();
}
fn mint_supply(svm: &LiteSVM, mint: &Pubkey) -> u64 {
    MintState::unpack(&svm.get_account(mint).unwrap().data)
        .unwrap()
        .supply
}
fn create_token_account(svm: &mut LiteSVM, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Pubkey {
    let addr = Pubkey::new_unique();
    write_token_account(svm, addr, mint, owner, amount);
    addr
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
fn token_amount(svm: &LiteSVM, addr: &Pubkey) -> u64 {
    TokenState::unpack(&svm.get_account(addr).unwrap().data)
        .unwrap()
        .amount
}

// ---- settlement setup ----
struct Env {
    mint: Pubkey,
    treasury: Pubkey,
    poster: Keypair,
    #[allow(dead_code)]
    dispute: Keypair,
}

fn init_distributor(
    svm: &mut LiteSVM,
    authority: &Keypair,
    dispute_window: i64,
    clawback_window: i64,
) -> Env {
    let mint = create_mint(svm);
    let poster = Keypair::new();
    svm.airdrop(&poster.pubkey(), 1_000_000_000).unwrap();
    let dispute = Keypair::new();
    let treasury = create_token_account(svm, &mint, &Pubkey::new_unique(), 0);

    let data = rewards_settlement::instruction::InitializeDistributor {
        dispute_window_seconds: dispute_window,
        clawback_window_seconds: clawback_window,
    }
    .data();
    let metas = rewards_settlement::accounts::InitializeDistributor {
        authority: authority.pubkey(),
        reward_mint: mint,
        poster_authority: poster.pubkey(),
        dispute_authority: dispute.pubkey(),
        treasury,
        distributor: distributor_pda(),
        reward_vault: vault_pda(),
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        authority,
        &[authority],
    )
    .unwrap();
    Env {
        mint,
        treasury,
        poster,
        dispute,
    }
}

/// Directly credit the vault (bypassing fund_vault) and mirror it into mint supply.
fn seed_vault(svm: &mut LiteSVM, env: &Env, amount: u64) {
    write_token_account(svm, vault_pda(), &env.mint, &distributor_pda(), amount);
    let new_supply = mint_supply(svm, &env.mint) + amount;
    set_mint_supply(svm, &env.mint, new_supply);
}

fn post_epoch(
    svm: &mut LiteSVM,
    env: &Env,
    epoch: u64,
    root: [u8; 32],
    total_reward: u64,
    num_nodes: u32,
) -> TransactionResult {
    let data = rewards_settlement::instruction::PostEpoch {
        epoch,
        merkle_root: root,
        total_reward,
        num_nodes,
    }
    .data();
    let metas = rewards_settlement::accounts::PostEpoch {
        poster: env.poster.pubkey(),
        distributor: distributor_pda(),
        reward_vault: vault_pda(),
        epoch_distribution: epoch_pda(epoch),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        &env.poster,
        &[&env.poster],
    )
}

#[allow(clippy::too_many_arguments)]
fn claim(
    svm: &mut LiteSVM,
    claimant: &Keypair,
    env: &Env,
    epoch: u64,
    operator: &Pubkey,
    operator_ta: &Pubkey,
    node_id: u64,
    amount: u64,
    proof: Vec<[u8; 32]>,
) -> TransactionResult {
    let data = rewards_settlement::instruction::Claim {
        epoch,
        node_id,
        amount,
        proof,
    }
    .data();
    let metas = rewards_settlement::accounts::Claim {
        claimant: claimant.pubkey(),
        distributor: distributor_pda(),
        epoch_distribution: epoch_pda(epoch),
        operator: *operator,
        claim_status: claim_pda(epoch, operator, node_id),
        reward_mint: env.mint,
        reward_vault: vault_pda(),
        operator_token_account: *operator_ta,
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        claimant,
        &[claimant],
    )
}

/// A small reward set: (operator, node_id, amount). Returns leaves in order.
fn build_tree(epoch: u64, entries: &[(Pubkey, u64, u64)]) -> Vec<[u8; 32]> {
    entries
        .iter()
        .map(|(op, id, amt)| hash_reward_leaf(epoch, &op.to_bytes(), *id, *amt))
        .collect()
}

fn distributor_obligated(svm: &LiteSVM) -> u64 {
    let d = svm.get_account(&distributor_pda()).unwrap();
    rewards_settlement::Distributor::try_deserialize(&mut d.data.as_slice())
        .unwrap()
        .cumulative_obligated
}

#[test]
fn pay_traffic_splits_70_20_10_and_burns() {
    let (mut svm, authority) = setup();
    let env = init_distributor(&mut svm, &authority, 100, 1000);

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000).unwrap();
    let amount = 1_000_000u64; // clean split
    let payer_ta = create_token_account(&mut svm, &env.mint, &payer.pubkey(), amount);
    let new_supply = mint_supply(&svm, &env.mint) + amount;
    set_mint_supply(&mut svm, &env.mint, new_supply);
    let supply_before = mint_supply(&svm, &env.mint);

    let data = rewards_settlement::instruction::PayTraffic { amount }.data();
    let metas = rewards_settlement::accounts::PayTraffic {
        payer: payer.pubkey(),
        distributor: distributor_pda(),
        reward_mint: env.mint,
        payer_token_account: payer_ta,
        reward_vault: vault_pda(),
        treasury: env.treasury,
        token_program: token_program_id(),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        &payer,
        &[&payer],
    )
    .unwrap();

    let split = split_payment(amount);
    assert_eq!(token_amount(&svm, &vault_pda()), split.nodes);
    assert_eq!(token_amount(&svm, &env.treasury), split.treasury);
    assert_eq!(token_amount(&svm, &payer_ta), 0);
    // burn reduced total supply by exactly the burn share
    assert_eq!(mint_supply(&svm, &env.mint), supply_before - split.burn);
    // 70/20/10 on a clean amount
    assert_eq!(split.nodes, 700_000);
    assert_eq!(split.burn, 200_000);
    assert_eq!(split.treasury, 100_000);
}

#[test]
fn pay_traffic_rounding_remainder_goes_to_treasury() {
    let (mut svm, authority) = setup();
    let env = init_distributor(&mut svm, &authority, 100, 1000);
    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 1_000_000_000).unwrap();
    let amount = 7u64; // 70%/20% floor → nodes=4, burn=1, treasury=2
    let payer_ta = create_token_account(&mut svm, &env.mint, &payer.pubkey(), amount);
    let new_supply = mint_supply(&svm, &env.mint) + amount;
    set_mint_supply(&mut svm, &env.mint, new_supply);

    let data = rewards_settlement::instruction::PayTraffic { amount }.data();
    let metas = rewards_settlement::accounts::PayTraffic {
        payer: payer.pubkey(),
        distributor: distributor_pda(),
        reward_mint: env.mint,
        payer_token_account: payer_ta,
        reward_vault: vault_pda(),
        treasury: env.treasury,
        token_program: token_program_id(),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
        &payer,
        &[&payer],
    )
    .unwrap();
    let split = split_payment(amount);
    assert_eq!(split.nodes + split.burn + split.treasury, amount);
    assert_eq!(token_amount(&svm, &vault_pda()), split.nodes);
    assert_eq!(token_amount(&svm, &env.treasury), split.treasury);
}

#[test]
fn post_epoch_requires_poster_and_solvency_and_monotonic() {
    let (mut svm, authority) = setup();
    let env = init_distributor(&mut svm, &authority, 100, 1000);
    set_clock(&mut svm, 1_000);

    let op = Pubkey::new_unique();
    let leaves = build_tree(1, &[(op, 7, 500_000)]);
    let root = merkle_root(&leaves);

    // not the poster → Unauthorized
    let imposter = Keypair::new();
    svm.airdrop(&imposter.pubkey(), 1_000_000_000).unwrap();
    let data = rewards_settlement::instruction::PostEpoch {
        epoch: 1,
        merkle_root: root,
        total_reward: 500_000,
        num_nodes: 1,
    }
    .data();
    let metas = rewards_settlement::accounts::PostEpoch {
        poster: imposter.pubkey(),
        distributor: distributor_pda(),
        reward_vault: vault_pda(),
        epoch_distribution: epoch_pda(1),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    assert_failed_with(
        send(
            &mut svm,
            Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
            &imposter,
            &[&imposter],
        ),
        "Unauthorized",
    );

    // poster, but vault is empty → InsufficientVault
    assert_failed_with(
        post_epoch(&mut svm, &env, 1, root, 500_000, 1),
        "InsufficientVault",
    );

    // fund the vault, then it posts
    seed_vault(&mut svm, &env, 500_000);
    post_epoch(&mut svm, &env, 1, root, 500_000, 1).unwrap();

    // re-posting epoch 1 fails (init seed collision / monotonic guard)
    assert!(post_epoch(&mut svm, &env, 1, root, 1, 1).is_err());

    // a stale/equal epoch number also fails the monotonic guard
    assert_failed_with(
        post_epoch(&mut svm, &env, 0, root, 0, 0),
        "NonMonotonicEpoch",
    );
}

#[test]
fn claim_pays_after_window_and_blocks_double_claim() {
    let (mut svm, authority) = setup();
    let env = init_distributor(&mut svm, &authority, 100, 1000);
    set_clock(&mut svm, 1_000);

    let op_a = Pubkey::new_unique();
    let op_b = Pubkey::new_unique();
    let op_c = Pubkey::new_unique();
    // 3-leaf tree exercises the odd-promote path
    let entries = [
        (op_a, 10, 300_000u64),
        (op_b, 11, 200_000),
        (op_c, 12, 150_000),
    ];
    let total: u64 = entries.iter().map(|e| e.2).sum();
    let leaves = build_tree(7, &entries);
    let root = merkle_root(&leaves);

    seed_vault(&mut svm, &env, total);
    post_epoch(&mut svm, &env, 7, root, total, 3).unwrap();

    let op_a_ta = create_token_account(&mut svm, &env.mint, &op_a, 0);
    let claimant = Keypair::new();
    svm.airdrop(&claimant.pubkey(), 1_000_000_000).unwrap();

    // before the dispute window elapses → DisputeWindowOpen
    let proof_a = merkle_proof(&leaves, 0);
    assert_failed_with(
        claim(
            &mut svm,
            &claimant,
            &env,
            7,
            &op_a,
            &op_a_ta,
            10,
            300_000,
            proof_a.clone(),
        ),
        "DisputeWindowOpen",
    );

    // advance past posted_at + dispute_window (1000 + 100)
    set_clock(&mut svm, 1_200);

    // wrong amount → InvalidProof
    assert_failed_with(
        claim(
            &mut svm,
            &claimant,
            &env,
            7,
            &op_a,
            &op_a_ta,
            10,
            999,
            proof_a.clone(),
        ),
        "InvalidProof",
    );

    // correct claim pays the operator ATA
    claim(
        &mut svm,
        &claimant,
        &env,
        7,
        &op_a,
        &op_a_ta,
        10,
        300_000,
        proof_a.clone(),
    )
    .unwrap();
    assert_eq!(token_amount(&svm, &op_a_ta), 300_000);
    assert_eq!(token_amount(&svm, &vault_pda()), total - 300_000);

    // ClaimStatus recorded
    let cs = svm.get_account(&claim_pda(7, &op_a, 10)).unwrap();
    let parsed = rewards_settlement::ClaimStatus::try_deserialize(&mut cs.data.as_slice()).unwrap();
    assert_eq!(parsed.amount, 300_000);
    assert_eq!(parsed.node_id, 10);
    assert!(!parsed.disputed);

    // double-claim → the init'd ClaimStatus PDA already exists
    assert!(claim(&mut svm, &claimant, &env, 7, &op_a, &op_a_ta, 10, 300_000, proof_a,).is_err());

    // a second distinct node still claims (proof for index 1)
    let op_b_ta = create_token_account(&mut svm, &env.mint, &op_b, 0);
    let proof_b = merkle_proof(&leaves, 1);
    claim(
        &mut svm, &claimant, &env, 7, &op_b, &op_b_ta, 11, 200_000, proof_b,
    )
    .unwrap();
    assert_eq!(token_amount(&svm, &op_b_ta), 200_000);
}

#[test]
fn sweep_epoch_deobligates_residual_after_clawback() {
    let (mut svm, authority) = setup();
    let env = init_distributor(&mut svm, &authority, 100, 1000);
    set_clock(&mut svm, 1_000);

    let op = Pubkey::new_unique();
    let entries = [(op, 5, 400_000u64)];
    let leaves = build_tree(2, &entries);
    let root = merkle_root(&leaves);
    seed_vault(&mut svm, &env, 400_000);
    post_epoch(&mut svm, &env, 2, root, 400_000, 1).unwrap();

    // before clawback window → ClawbackWindowOpen
    let data = rewards_settlement::instruction::SweepEpoch { _epoch: 2 }.data();
    let metas = rewards_settlement::accounts::SweepEpoch {
        authority: authority.pubkey(),
        distributor: distributor_pda(),
        epoch_distribution: epoch_pda(2),
    }
    .to_account_metas(None);
    assert_failed_with(
        send(
            &mut svm,
            Instruction::new_with_bytes(rewards_settlement::ID, &data, metas.clone()),
            &authority,
            &[&authority],
        ),
        "ClawbackWindowOpen",
    );

    // claim part of it, then sweep the rest
    set_clock(&mut svm, 1_200);
    let op_ta = create_token_account(&mut svm, &env.mint, &op, 0);
    claim(
        &mut svm,
        &authority,
        &env,
        2,
        &op,
        &op_ta,
        5,
        400_000,
        merkle_proof(&leaves, 0),
    )
    .unwrap();

    // advance past clawback (posted_at 1000 + 1000)
    set_clock(&mut svm, 2_100);
    let d_before = distributor_obligated(&svm);
    send(
        &mut svm,
        Instruction::new_with_bytes(rewards_settlement::ID, &data, metas.clone()),
        &authority,
        &[&authority],
    )
    .unwrap();
    // fully claimed → residual 0, obligated unchanged but epoch marked swept
    assert_eq!(distributor_obligated(&svm), d_before);

    // double sweep → AlreadySwept
    assert_failed_with(
        send(
            &mut svm,
            Instruction::new_with_bytes(rewards_settlement::ID, &data, metas),
            &authority,
            &[&authority],
        ),
        "AlreadySwept",
    );
}
