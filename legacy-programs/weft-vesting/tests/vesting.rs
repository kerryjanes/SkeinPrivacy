//! LiteSVM integration tests for the `weft-vesting` program.
//!
//! Mint and token accounts are seeded directly via `set_account` with packed
//! SPL state (the litesvm-recommended pattern); the schedule clock is moved with
//! `set_sysvar::<Clock>` to exercise cliff / linear / full-vest / revoke paths.

// `send` returns litesvm's `TransactionResult`, whose Err variant is large.
#![allow(clippy::result_large_err)]

use {
    anchor_lang::{
        solana_program::instruction::Instruction, AccountDeserialize, InstructionData,
        ToAccountMetas,
    },
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

const PROGRAM_BYTES: &[u8] = include_bytes!("../../../target/deploy/weft_vesting.so");
const DECIMALS: u8 = 9;
const TOTAL: u64 = 1_000_000;

fn token_program_id() -> Pubkey {
    spl_token_interface::ID
}

fn setup() -> LiteSVM {
    let mut svm = LiteSVM::new();
    svm.add_program(weft_vesting::ID, PROGRAM_BYTES).unwrap();
    svm
}

fn set_clock(svm: &mut LiteSVM, unix_ts: i64) {
    let mut clock: Clock = svm.get_sysvar();
    clock.unix_timestamp = unix_ts;
    svm.set_sysvar(&clock);
}

fn funded_keypair(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 100_000_000_000).unwrap();
    kp
}

fn create_mint(svm: &mut LiteSVM) -> Pubkey {
    let mint = Pubkey::new_unique();
    let state = MintState {
        mint_authority: COption::Some(Pubkey::new_unique()),
        supply: 0,
        decimals: DECIMALS,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    let mut data = vec![0u8; MintState::LEN];
    MintState::pack(state, &mut data).unwrap();
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

fn create_token_account(svm: &mut LiteSVM, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Pubkey {
    let addr = Pubkey::new_unique();
    let state = TokenState {
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
    TokenState::pack(state, &mut data).unwrap();
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
    addr
}

fn token_amount(svm: &LiteSVM, addr: &Pubkey) -> u64 {
    let acc = svm.get_account(addr).unwrap();
    TokenState::unpack(&acc.data).unwrap().amount
}

fn schedule_pda(beneficiary: &Pubkey, schedule_id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[
            weft_vesting::SCHEDULE_SEED,
            beneficiary.as_ref(),
            &schedule_id.to_le_bytes(),
        ],
        &weft_vesting::ID,
    )
    .0
}

fn vault_pda(schedule: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[weft_vesting::VAULT_SEED, schedule.as_ref()],
        &weft_vesting::ID,
    )
    .0
}

fn get_schedule(svm: &LiteSVM, pda: &Pubkey) -> weft_vesting::VestingSchedule {
    let acc = svm.get_account(pda).unwrap();
    weft_vesting::VestingSchedule::try_deserialize(&mut acc.data.as_slice()).unwrap()
}

fn send(
    svm: &mut LiteSVM,
    ix: Instruction,
    payer: &Keypair,
    signers: &[&Keypair],
) -> TransactionResult {
    // Advance the blockhash so otherwise-identical txs get distinct signatures
    // (LiteSVM rejects a replayed signature with `AlreadyProcessed`).
    svm.expire_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &svm.latest_blockhash());
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    svm.send_transaction(tx)
}

fn assert_failed_with(res: TransactionResult, code: &str) {
    let failed = res.expect_err("expected the transaction to fail");
    let logs = failed.meta.logs.join("\n");
    assert!(
        logs.contains(code),
        "expected error `{code}`, got logs:\n{logs}"
    );
}

#[allow(clippy::too_many_arguments)]
fn create_schedule_ix(
    funder: &Pubkey,
    beneficiary: &Pubkey,
    authority: &Pubkey,
    mint: &Pubkey,
    funder_ta: &Pubkey,
    schedule_id: u64,
    total_amount: u64,
    cliff_unlock_amount: u64,
    start_ts: i64,
    cliff_duration: i64,
    duration: i64,
    revocable: bool,
) -> Instruction {
    let schedule = schedule_pda(beneficiary, schedule_id);
    let vault = vault_pda(&schedule);
    let data = weft_vesting::instruction::CreateSchedule {
        schedule_id,
        total_amount,
        cliff_unlock_amount,
        start_ts,
        cliff_duration,
        duration,
        revocable,
    }
    .data();
    let metas = weft_vesting::accounts::CreateSchedule {
        funder: *funder,
        beneficiary: *beneficiary,
        authority: *authority,
        mint: *mint,
        schedule,
        vault,
        funder_token_account: *funder_ta,
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(weft_vesting::ID, &data, metas)
}

fn claim_ix(
    beneficiary: &Pubkey,
    mint: &Pubkey,
    schedule: &Pubkey,
    vault: &Pubkey,
    destination: &Pubkey,
) -> Instruction {
    let data = weft_vesting::instruction::Claim {}.data();
    let metas = weft_vesting::accounts::Claim {
        beneficiary: *beneficiary,
        schedule: *schedule,
        mint: *mint,
        vault: *vault,
        destination: *destination,
        token_program: token_program_id(),
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(weft_vesting::ID, &data, metas)
}

fn revoke_ix(
    authority: &Pubkey,
    mint: &Pubkey,
    schedule: &Pubkey,
    vault: &Pubkey,
    destination: &Pubkey,
) -> Instruction {
    let data = weft_vesting::instruction::Revoke {}.data();
    let metas = weft_vesting::accounts::Revoke {
        authority: *authority,
        schedule: *schedule,
        mint: *mint,
        vault: *vault,
        destination: *destination,
        token_program: token_program_id(),
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(weft_vesting::ID, &data, metas)
}

/// Common fixture: a funded funder with a token account holding `total`, a fresh
/// mint, and a beneficiary/authority. Returns the created schedule + vault PDAs.
struct Fixture {
    svm: LiteSVM,
    beneficiary: Keypair,
    authority: Keypair,
    mint: Pubkey,
    schedule: Pubkey,
    vault: Pubkey,
}

#[allow(clippy::too_many_arguments)]
fn create(
    schedule_id: u64,
    total: u64,
    cliff_unlock: u64,
    start_ts: i64,
    cliff: i64,
    duration: i64,
    revocable: bool,
) -> Fixture {
    let mut svm = setup();
    let funder = funded_keypair(&mut svm);
    // Fund beneficiary and authority too: they pay fees when they sign claim/revoke.
    let beneficiary = funded_keypair(&mut svm);
    let authority = funded_keypair(&mut svm);
    let mint = create_mint(&mut svm);
    let funder_ta = create_token_account(&mut svm, &mint, &funder.pubkey(), total);

    let ix = create_schedule_ix(
        &funder.pubkey(),
        &beneficiary.pubkey(),
        &authority.pubkey(),
        &mint,
        &funder_ta,
        schedule_id,
        total,
        cliff_unlock,
        start_ts,
        cliff,
        duration,
        revocable,
    );
    send(&mut svm, ix, &funder, &[&funder]).expect("create_schedule should succeed");

    let schedule = schedule_pda(&beneficiary.pubkey(), schedule_id);
    let vault = vault_pda(&schedule);
    Fixture {
        svm,
        beneficiary,
        authority,
        mint,
        schedule,
        vault,
    }
}

#[test]
fn create_funds_vault_exactly() {
    let f = create(0, TOTAL, 0, 1_000, 0, 100, false);
    assert_eq!(token_amount(&f.svm, &f.vault), TOTAL);
    let s = get_schedule(&f.svm, &f.schedule);
    assert_eq!(s.total_amount, TOTAL);
    assert_eq!(s.released_amount, 0);
    assert_eq!(s.beneficiary, f.beneficiary.pubkey());
    assert_eq!(s.authority, f.authority.pubkey());
    assert!(!s.revoked);
}

#[test]
fn pre_cliff_nothing_claimable() {
    let mut f = create(0, TOTAL, 0, 1_000, 20, 100, false);
    set_clock(&mut f.svm, 1_010); // before cliff
    let dest = create_token_account(&mut f.svm, &f.mint, &f.beneficiary.pubkey(), 0);
    let ix = claim_ix(
        &f.beneficiary.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &dest,
    );
    let res = send(&mut f.svm, ix, &f.beneficiary, &[&f.beneficiary]);
    assert_failed_with(res, "NothingToClaim");
    assert_eq!(token_amount(&f.svm, &f.vault), TOTAL);
}

#[test]
fn cliff_boundary_releases_lump() {
    let mut f = create(0, TOTAL, 0, 1_000, 20, 100, false);
    set_clock(&mut f.svm, 1_020); // exactly at cliff → 20/100 vested
    let dest = create_token_account(&mut f.svm, &f.mint, &f.beneficiary.pubkey(), 0);
    let ix = claim_ix(
        &f.beneficiary.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &dest,
    );
    send(&mut f.svm, ix, &f.beneficiary, &[&f.beneficiary]).unwrap();
    assert_eq!(token_amount(&f.svm, &dest), 200_000);
    assert_eq!(token_amount(&f.svm, &f.vault), 800_000);
    assert_eq!(get_schedule(&f.svm, &f.schedule).released_amount, 200_000);
}

#[test]
fn linear_midpoint_no_cliff() {
    let mut f = create(0, TOTAL, 0, 1_000, 0, 100, false);
    set_clock(&mut f.svm, 1_050);
    let dest = create_token_account(&mut f.svm, &f.mint, &f.beneficiary.pubkey(), 0);
    let ix = claim_ix(
        &f.beneficiary.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &dest,
    );
    send(&mut f.svm, ix, &f.beneficiary, &[&f.beneficiary]).unwrap();
    assert_eq!(token_amount(&f.svm, &dest), 500_000);
}

#[test]
fn incremental_claims_are_monotonic_and_sweep() {
    let mut f = create(0, TOTAL, 0, 1_000, 0, 100, false);
    let dest = create_token_account(&mut f.svm, &f.mint, &f.beneficiary.pubkey(), 0);

    for (ts, expected_total) in [(1_025u64, 250_000u64), (1_050, 500_000), (1_200, 1_000_000)] {
        set_clock(&mut f.svm, ts as i64);
        let ix = claim_ix(
            &f.beneficiary.pubkey(),
            &f.mint,
            &f.schedule,
            &f.vault,
            &dest,
        );
        send(&mut f.svm, ix, &f.beneficiary, &[&f.beneficiary]).unwrap();
        assert_eq!(token_amount(&f.svm, &dest), expected_total);
        assert_eq!(
            get_schedule(&f.svm, &f.schedule).released_amount,
            expected_total
        );
    }
    // Fully swept, vault empty, no dust.
    assert_eq!(token_amount(&f.svm, &f.vault), 0);
}

#[test]
fn double_claim_same_time_fails() {
    let mut f = create(0, TOTAL, 0, 1_000, 0, 100, false);
    set_clock(&mut f.svm, 1_050);
    let dest = create_token_account(&mut f.svm, &f.mint, &f.beneficiary.pubkey(), 0);
    let ix = claim_ix(
        &f.beneficiary.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &dest,
    );
    send(&mut f.svm, ix.clone(), &f.beneficiary, &[&f.beneficiary]).unwrap();
    let res = send(&mut f.svm, ix, &f.beneficiary, &[&f.beneficiary]);
    assert_failed_with(res, "NothingToClaim");
}

#[test]
fn unauthorized_claim_fails() {
    let mut f = create(0, TOTAL, 0, 1_000, 0, 100, false);
    set_clock(&mut f.svm, 1_050);
    let attacker = funded_keypair(&mut f.svm);
    let dest = create_token_account(&mut f.svm, &f.mint, &attacker.pubkey(), 0);
    // Passing the real schedule PDA but the attacker as `beneficiary` breaks the
    // seeds derivation (seeds include beneficiary.key()).
    let ix = claim_ix(&attacker.pubkey(), &f.mint, &f.schedule, &f.vault, &dest);
    let res = send(&mut f.svm, ix, &attacker, &[&attacker]);
    assert!(res.is_err());
    assert_eq!(token_amount(&f.svm, &dest), 0);
}

#[test]
fn revoke_splits_and_conserves_vault() {
    let mut f = create(0, TOTAL, 0, 1_000, 20, 100, true);
    set_clock(&mut f.svm, 1_060); // 60/100 vested = 600_000
    let auth_dest = create_token_account(&mut f.svm, &f.mint, &f.authority.pubkey(), 0);
    let ix = revoke_ix(
        &f.authority.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &auth_dest,
    );
    send(&mut f.svm, ix, &f.authority, &[&f.authority]).unwrap();

    // Authority reclaims the unvested 400_000.
    assert_eq!(token_amount(&f.svm, &auth_dest), 400_000);
    let s = get_schedule(&f.svm, &f.schedule);
    assert!(s.revoked);
    assert_eq!(s.revoked_ts, 1_060);

    // Beneficiary can still claim the vested-but-unreleased 600_000.
    let ben_dest = create_token_account(&mut f.svm, &f.mint, &f.beneficiary.pubkey(), 0);
    let ix = claim_ix(
        &f.beneficiary.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &ben_dest,
    );
    send(&mut f.svm, ix, &f.beneficiary, &[&f.beneficiary]).unwrap();
    assert_eq!(token_amount(&f.svm, &ben_dest), 600_000);
    assert_eq!(token_amount(&f.svm, &f.vault), 0); // conserved
}

#[test]
fn revoke_freezes_accrual() {
    let mut f = create(0, TOTAL, 0, 1_000, 0, 100, true);
    set_clock(&mut f.svm, 1_040); // 40% vested
    let auth_dest = create_token_account(&mut f.svm, &f.mint, &f.authority.pubkey(), 0);
    let ix = revoke_ix(
        &f.authority.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &auth_dest,
    );
    send(&mut f.svm, ix, &f.authority, &[&f.authority]).unwrap();

    // Long after full duration, beneficiary still capped at the frozen 40%.
    set_clock(&mut f.svm, 5_000);
    let ben_dest = create_token_account(&mut f.svm, &f.mint, &f.beneficiary.pubkey(), 0);
    let ix = claim_ix(
        &f.beneficiary.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &ben_dest,
    );
    send(&mut f.svm, ix, &f.beneficiary, &[&f.beneficiary]).unwrap();
    assert_eq!(token_amount(&f.svm, &ben_dest), 400_000);
}

#[test]
fn revoke_non_revocable_fails() {
    let mut f = create(0, TOTAL, 0, 1_000, 0, 100, false);
    set_clock(&mut f.svm, 1_050);
    let auth_dest = create_token_account(&mut f.svm, &f.mint, &f.authority.pubkey(), 0);
    let ix = revoke_ix(
        &f.authority.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &auth_dest,
    );
    let res = send(&mut f.svm, ix, &f.authority, &[&f.authority]);
    assert_failed_with(res, "NotRevocable");
}

#[test]
fn double_revoke_fails() {
    let mut f = create(0, TOTAL, 0, 1_000, 0, 100, true);
    set_clock(&mut f.svm, 1_050);
    let auth_dest = create_token_account(&mut f.svm, &f.mint, &f.authority.pubkey(), 0);
    let ix = revoke_ix(
        &f.authority.pubkey(),
        &f.mint,
        &f.schedule,
        &f.vault,
        &auth_dest,
    );
    send(&mut f.svm, ix.clone(), &f.authority, &[&f.authority]).unwrap();
    let res = send(&mut f.svm, ix, &f.authority, &[&f.authority]);
    assert_failed_with(res, "AlreadyRevoked");
}

#[test]
fn unauthorized_revoke_fails() {
    let mut f = create(0, TOTAL, 0, 1_000, 0, 100, true);
    set_clock(&mut f.svm, 1_050);
    let attacker = funded_keypair(&mut f.svm);
    let dest = create_token_account(&mut f.svm, &f.mint, &attacker.pubkey(), 0);
    let ix = revoke_ix(&attacker.pubkey(), &f.mint, &f.schedule, &f.vault, &dest);
    let res = send(&mut f.svm, ix, &attacker, &[&attacker]);
    assert_failed_with(res, "Unauthorized");
}

#[test]
fn create_rejects_invalid_params() {
    // duration = 0
    let mut svm = setup();
    let funder = funded_keypair(&mut svm);
    let beneficiary = Keypair::new();
    let mint = create_mint(&mut svm);
    let funder_ta = create_token_account(&mut svm, &mint, &funder.pubkey(), TOTAL);
    let ix = create_schedule_ix(
        &funder.pubkey(),
        &beneficiary.pubkey(),
        &beneficiary.pubkey(),
        &mint,
        &funder_ta,
        0,
        TOTAL,
        0,
        1_000,
        0,
        0, // duration 0 → invalid
        false,
    );
    assert_failed_with(send(&mut svm, ix, &funder, &[&funder]), "InvalidParams");

    // cliff > duration
    let ix = create_schedule_ix(
        &funder.pubkey(),
        &beneficiary.pubkey(),
        &beneficiary.pubkey(),
        &mint,
        &funder_ta,
        1,
        TOTAL,
        0,
        1_000,
        200,
        100, // cliff 200 > duration 100
        false,
    );
    assert_failed_with(send(&mut svm, ix, &funder, &[&funder]), "InvalidParams");
}

#[test]
fn team_schedule_matches_spec_at_cliff() {
    // 150M $WEFT, 12-month cliff, 36-month vesting → exactly 50M at the cliff.
    let total = weft_primitives::allocation_amount(weft_primitives::allocation_bps::TEAM);
    let cliff = weft_primitives::vesting::TEAM_CLIFF;
    let duration = weft_primitives::vesting::TEAM_DURATION;

    let mut svm = setup();
    let funder = funded_keypair(&mut svm);
    let beneficiary = funded_keypair(&mut svm);
    let mint = create_mint(&mut svm);
    let funder_ta = create_token_account(&mut svm, &mint, &funder.pubkey(), total);
    let ix = create_schedule_ix(
        &funder.pubkey(),
        &beneficiary.pubkey(),
        &funder.pubkey(),
        &mint,
        &funder_ta,
        0,
        total,
        0,
        0,
        cliff,
        duration,
        true,
    );
    send(&mut svm, ix, &funder, &[&funder]).unwrap();

    let schedule = schedule_pda(&beneficiary.pubkey(), 0);
    let vault = vault_pda(&schedule);
    set_clock(&mut svm, cliff); // start_ts = 0, so now == cliff
    let dest = create_token_account(&mut svm, &mint, &beneficiary.pubkey(), 0);
    let ix = claim_ix(&beneficiary.pubkey(), &mint, &schedule, &vault, &dest);
    send(&mut svm, ix, &beneficiary, &[&beneficiary]).unwrap();
    assert_eq!(
        token_amount(&svm, &dest),
        50_000_000 * weft_primitives::ONE_WEFT
    );
}
