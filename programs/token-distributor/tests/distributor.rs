//! Multi-program LiteSVM tests for the IDO/TGE distributor: load token-distributor +
//! weft-vesting, post a small allocation tree, and prove each claim credits the 25% TGE
//! portion to the claimant and creates a 75% 12-month vesting schedule via CPI — plus
//! double-claim / bad-proof / before-TGE rejection and that the vested 75% actually
//! releases from weft-vesting after time advances.

#![allow(clippy::result_large_err)]

use {
    anchor_lang::{
        solana_program::instruction::Instruction, AccountDeserialize, InstructionData,
        ToAccountMetas,
    },
    litesvm::{types::TransactionResult, LiteSVM},
    weft_primitives::{
        merkle::{hash_allocation_leaf, merkle_proof, merkle_root},
        split_tge, TGE_UNLOCK_BPS,
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

const DISTRIBUTOR_BYTES: &[u8] = include_bytes!("../../../target/deploy/token_distributor.so");
const VESTING_BYTES: &[u8] = include_bytes!("../../../target/deploy/weft_vesting.so");
const DECIMALS: u8 = 9;
const TGE_TS: i64 = 1_000_000;
const VESTING_DURATION: i64 = 365 * 24 * 3600; // 12 months

fn token_program_id() -> Pubkey {
    spl_token_interface::ID
}

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(token_distributor::ID, DISTRIBUTOR_BYTES)
        .unwrap();
    svm.add_program(weft_vesting::ID, VESTING_BYTES).unwrap();
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
    Pubkey::find_program_address(&[token_distributor::IDO_SEED], &token_distributor::ID).0
}
fn ido_vault_pda() -> Pubkey {
    Pubkey::find_program_address(&[b"ido_vault"], &token_distributor::ID).0
}
fn alloc_pda(claimant: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[token_distributor::ALLOC_SEED, claimant.as_ref()],
        &token_distributor::ID,
    )
    .0
}
fn schedule_pda(beneficiary: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            weft_vesting::SCHEDULE_SEED,
            beneficiary.as_ref(),
            &0u64.to_le_bytes(),
        ],
        &weft_vesting::ID,
    )
    .0
}
fn schedule_vault_pda(schedule: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[weft_vesting::VAULT_SEED, schedule.as_ref()],
        &weft_vesting::ID,
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

fn init_ido(svm: &mut LiteSVM, authority: &Keypair, mint: &Pubkey, root: [u8; 32], total: u64) {
    let data = token_distributor::instruction::InitializeIdo {
        merkle_root: root,
        tge_ts: TGE_TS,
        tge_bps: TGE_UNLOCK_BPS,
        vesting_duration: VESTING_DURATION,
        total_allocation: total,
    }
    .data();
    let metas = token_distributor::accounts::InitializeIdo {
        authority: authority.pubkey(),
        mint: *mint,
        distributor: distributor_pda(),
        vault: ido_vault_pda(),
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(token_distributor::ID, &data, metas),
        authority,
        &[authority],
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn claim(
    svm: &mut LiteSVM,
    claimant: &Keypair,
    mint: &Pubkey,
    claimant_ta: &Pubkey,
    amount: u64,
    proof: Vec<[u8; 32]>,
) -> TransactionResult {
    let data = token_distributor::instruction::Claim { amount, proof }.data();
    let schedule = schedule_pda(&claimant.pubkey());
    let metas = token_distributor::accounts::Claim {
        claimant: claimant.pubkey(),
        distributor: distributor_pda(),
        mint: *mint,
        vault: ido_vault_pda(),
        claimant_token_account: *claimant_ta,
        allocation_claim: alloc_pda(&claimant.pubkey()),
        vesting_program: weft_vesting::ID,
        schedule,
        schedule_vault: schedule_vault_pda(&schedule),
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(token_distributor::ID, &data, metas),
        claimant,
        &[claimant],
    )
}

/// (claimant, amount) allocations → the leaves.
fn build_tree(dist: &Pubkey, entries: &[(Pubkey, u64)]) -> Vec<[u8; 32]> {
    entries
        .iter()
        .map(|(c, a)| hash_allocation_leaf(&dist.to_bytes(), &c.to_bytes(), *a))
        .collect()
}

#[test]
fn claim_credits_tge_and_creates_vesting_schedule() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, TGE_TS + 10);
    let mint = create_mint(&mut svm);

    let a = Keypair::new();
    let b = Keypair::new();
    let c = Keypair::new();
    for kp in [&a, &b, &c] {
        svm.airdrop(&kp.pubkey(), 1_000_000_000).unwrap();
    }
    let entries = [
        (a.pubkey(), 4_000_000_000u64),
        (b.pubkey(), 2_000_000_000),
        (c.pubkey(), 1_000_000_000),
    ];
    let total: u64 = entries.iter().map(|e| e.1).sum();
    let dist = distributor_pda();
    let leaves = build_tree(&dist, &entries);
    let root = merkle_root(&leaves);

    init_ido(&mut svm, &authority, &mint, root, total);
    // fund the IDO vault with the full allocation.
    write_token_account(&mut svm, ido_vault_pda(), &mint, &dist, total);

    // claimant A claims index 0.
    let a_ta = create_token_account(&mut svm, &mint, &a.pubkey(), 0);
    let proof_a = merkle_proof(&leaves, 0);
    // wrong amount → InvalidProof
    assert_failed_with(
        claim(&mut svm, &a, &mint, &a_ta, 999, proof_a.clone()),
        "InvalidProof",
    );
    claim(&mut svm, &a, &mint, &a_ta, entries[0].1, proof_a.clone()).unwrap();

    let (tge, vest) = split_tge(entries[0].1, TGE_UNLOCK_BPS);
    assert_eq!(tge, 1_000_000_000); // 25%
    assert_eq!(vest, 3_000_000_000); // 75%
    assert_eq!(token_amount(&svm, &a_ta), tge); // TGE liquid in the ATA

    // the 75% vesting schedule was created via CPI.
    let sched = schedule_pda(&a.pubkey());
    let vs = weft_vesting::VestingSchedule::try_deserialize(
        &mut svm.get_account(&sched).unwrap().data.as_slice(),
    )
    .unwrap();
    assert_eq!(vs.beneficiary, a.pubkey());
    assert_eq!(vs.total_amount, vest);
    assert_eq!(vs.duration, VESTING_DURATION);
    assert_eq!(vs.start_ts, TGE_TS);
    assert!(!vs.revocable);
    assert_eq!(token_amount(&svm, &schedule_vault_pda(&sched)), vest);

    // double-claim rejected (AllocationClaim PDA already exists).
    assert!(claim(&mut svm, &a, &mint, &a_ta, entries[0].1, proof_a).is_err());

    // a second claimant still claims.
    let b_ta = create_token_account(&mut svm, &mint, &b.pubkey(), 0);
    claim(
        &mut svm,
        &b,
        &mint,
        &b_ta,
        entries[1].1,
        merkle_proof(&leaves, 1),
    )
    .unwrap();
    assert_eq!(
        token_amount(&svm, &b_ta),
        split_tge(entries[1].1, TGE_UNLOCK_BPS).0
    );

    // distributor total_claimed tracks both.
    let d = token_distributor::IdoDistributor::try_deserialize(
        &mut svm.get_account(&distributor_pda()).unwrap().data.as_slice(),
    )
    .unwrap();
    assert_eq!(d.total_claimed, entries[0].1 + entries[1].1);
}

#[test]
fn claim_rejected_before_tge() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, TGE_TS - 100); // before TGE
    let mint = create_mint(&mut svm);
    let a = Keypair::new();
    svm.airdrop(&a.pubkey(), 1_000_000_000).unwrap();
    let entries = [(a.pubkey(), 4_000_000_000u64)];
    let total = entries[0].1;
    let dist = distributor_pda();
    let leaves = build_tree(&dist, &entries);
    init_ido(&mut svm, &authority, &mint, merkle_root(&leaves), total);
    write_token_account(&mut svm, ido_vault_pda(), &mint, &dist, total);
    let a_ta = create_token_account(&mut svm, &mint, &a.pubkey(), 0);
    assert_failed_with(
        claim(&mut svm, &a, &mint, &a_ta, total, merkle_proof(&leaves, 0)),
        "BeforeTge",
    );
}

#[test]
fn vested_portion_releases_after_duration() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, TGE_TS + 10);
    let mint = create_mint(&mut svm);
    let a = Keypair::new();
    svm.airdrop(&a.pubkey(), 1_000_000_000).unwrap();
    let entries = [(a.pubkey(), 4_000_000_000u64)];
    let total = entries[0].1;
    let dist = distributor_pda();
    let leaves = build_tree(&dist, &entries);
    init_ido(&mut svm, &authority, &mint, merkle_root(&leaves), total);
    write_token_account(&mut svm, ido_vault_pda(), &mint, &dist, total);
    let a_ta = create_token_account(&mut svm, &mint, &a.pubkey(), 0);
    claim(&mut svm, &a, &mint, &a_ta, total, merkle_proof(&leaves, 0)).unwrap();

    let (_tge, vest) = split_tge(total, TGE_UNLOCK_BPS);
    let sched = schedule_pda(&a.pubkey());

    // warp fully past the vesting duration and claim the vested remainder.
    set_clock(&mut svm, TGE_TS + VESTING_DURATION + 1);
    let dest = create_token_account(&mut svm, &mint, &a.pubkey(), 0);
    let data = weft_vesting::instruction::Claim {}.data();
    let metas = weft_vesting::accounts::Claim {
        beneficiary: a.pubkey(),
        schedule: sched,
        mint,
        vault: schedule_vault_pda(&sched),
        destination: dest,
        token_program: token_program_id(),
    }
    .to_account_metas(None);
    send(
        &mut svm,
        Instruction::new_with_bytes(weft_vesting::ID, &data, metas),
        &a,
        &[&a],
    )
    .unwrap();
    // the full 75% is now released to the destination.
    assert_eq!(token_amount(&svm, &dest), vest);
}
