// Regression armor for the delegated-settlement refactor.
//
// `settle_from_escrow` is what makes "use = earn" seamless: the relay/aggregator (the
// distributor's `poster_authority`) bills a user's metered VPN usage straight from their prepaid
// escrow — no per-charge owner signature — and the charge is split 70% reward-vault / 20% burn /
// 10% treasury, exactly like the owner-signed `pay_traffic_from_escrow`. These tests run the built
// program in LiteSVM against Token-2022 accounts and prove:
//   1. the happy path moves 70/10 into vault/treasury, burns 20% (mint supply drops), and debits
//      the escrow balance;
//   2. only the poster authority may settle (a stranger is rejected);
//   3. the charge can never exceed the escrow's own balance (no overdraw).
use anchor_lang::solana_program::{instruction::Instruction, pubkey::Pubkey, system_instruction};
use anchor_lang::{AccountDeserialize, AccountSerialize, InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;

use anchor_spl::token_2022::spl_token_2022::{
    extension::StateWithExtensions,
    instruction as token_ix,
    state::{Account as TokenState, Mint as MintState},
    ID as TOKEN_2022,
};

const SO: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/deploy/weft.so"));

// Token-2022 base (no-extension) account sizes.
const MINT_LEN: usize = 82;
const ACCOUNT_LEN: usize = 165;

const DEPOSIT: u64 = 1_000_000; // 1.0 token at 6 decimals

fn send(svm: &mut LiteSVM, ixs: &[Instruction], payer: &Keypair, signers: &[&Keypair]) {
    let msg = Message::new(ixs, Some(&payer.pubkey()));
    let tx = Transaction::new(signers, msg, svm.latest_blockhash());
    svm.send_transaction(tx).unwrap();
}

fn create_mint(svm: &mut LiteSVM, payer: &Keypair, mint: &Keypair, authority: &Pubkey) {
    let rent = svm.minimum_balance_for_rent_exemption(MINT_LEN);
    let create = system_instruction::create_account(
        &payer.pubkey(),
        &mint.pubkey(),
        rent,
        MINT_LEN as u64,
        &TOKEN_2022,
    );
    let init = token_ix::initialize_mint2(&TOKEN_2022, &mint.pubkey(), authority, None, 6).unwrap();
    send(svm, &[create, init], payer, &[payer, mint]);
}

fn create_token_account(
    svm: &mut LiteSVM,
    payer: &Keypair,
    account: &Keypair,
    mint: &Pubkey,
    authority: &Pubkey,
) {
    let rent = svm.minimum_balance_for_rent_exemption(ACCOUNT_LEN);
    let create = system_instruction::create_account(
        &payer.pubkey(),
        &account.pubkey(),
        rent,
        ACCOUNT_LEN as u64,
        &TOKEN_2022,
    );
    let init =
        token_ix::initialize_account3(&TOKEN_2022, &account.pubkey(), mint, authority).unwrap();
    send(svm, &[create, init], payer, &[payer, account]);
}

fn mint_to(svm: &mut LiteSVM, payer: &Keypair, mint: &Pubkey, dest: &Pubkey, amount: u64) {
    let ix = token_ix::mint_to(&TOKEN_2022, mint, dest, &payer.pubkey(), &[], amount).unwrap();
    send(svm, &[ix], payer, &[payer]);
}

fn set_anchor_account<T: AccountSerialize>(svm: &mut LiteSVM, addr: &Pubkey, owner: &Pubkey, acct: &T) {
    let mut data = Vec::new();
    acct.try_serialize(&mut data).unwrap();
    let lamports = svm.minimum_balance_for_rent_exemption(data.len());
    svm.set_account(
        *addr,
        Account { lamports, data, owner: *owner, executable: false, rent_epoch: 0 },
    )
    .unwrap();
}

fn token_amount(svm: &LiteSVM, addr: &Pubkey) -> u64 {
    let acct = svm.get_account(addr).unwrap();
    StateWithExtensions::<TokenState>::unpack(&acct.data).unwrap().base.amount
}

fn mint_supply(svm: &LiteSVM, addr: &Pubkey) -> u64 {
    let acct = svm.get_account(addr).unwrap();
    StateWithExtensions::<MintState>::unpack(&acct.data).unwrap().base.supply
}

fn escrow_balance(svm: &LiteSVM, addr: &Pubkey) -> (u64, u64) {
    let acct = svm.get_account(addr).unwrap();
    let e = weft::PaymentEscrow::try_deserialize(&mut acct.data.as_slice()).unwrap();
    (e.balance, e.total_spent)
}

struct Env {
    svm: LiteSVM,
    poster: Keypair,
    mint: Pubkey,
    distributor: Pubkey,
    escrow: Pubkey,
    escrow_vault: Pubkey,
    reward_vault: Pubkey,
    treasury: Pubkey,
}

/// Build a fully-provisioned world seeded to the moment just before settlement:
/// a Token-2022 mint, a funded escrow_vault (authority = escrow PDA), empty vault + treasury,
/// and the Distributor + PaymentEscrow program accounts at their canonical PDAs.
fn setup() -> Env {
    let mut svm = LiteSVM::new();
    svm.add_program(weft::ID, SO).unwrap();

    let poster = Keypair::new();
    svm.airdrop(&poster.pubkey(), 100_000_000_000).unwrap();

    let owner = Pubkey::new_unique(); // the VPN user; need not sign for delegated settlement

    let (distributor, dist_bump) =
        Pubkey::find_program_address(&[weft::DISTRIBUTOR_SEED], &weft::ID);
    let (escrow, escrow_bump) =
        Pubkey::find_program_address(&[weft::ESCROW_SEED, owner.as_ref()], &weft::ID);

    let mint_kp = Keypair::new();
    create_mint(&mut svm, &poster, &mint_kp, &poster.pubkey());
    let mint = mint_kp.pubkey();

    let escrow_vault_kp = Keypair::new();
    create_token_account(&mut svm, &poster, &escrow_vault_kp, &mint, &escrow);
    let escrow_vault = escrow_vault_kp.pubkey();
    mint_to(&mut svm, &poster, &mint, &escrow_vault, DEPOSIT);

    let reward_vault_kp = Keypair::new();
    create_token_account(&mut svm, &poster, &reward_vault_kp, &mint, &distributor);
    let reward_vault = reward_vault_kp.pubkey();

    let treasury_kp = Keypair::new();
    create_token_account(&mut svm, &poster, &treasury_kp, &mint, &distributor);
    let treasury = treasury_kp.pubkey();

    let dist = weft::Distributor {
        authority: poster.pubkey(),
        poster_authority: poster.pubkey(),
        dispute_authority: poster.pubkey(),
        reward_mint: mint,
        reward_vault,
        treasury,
        dispute_window_seconds: 600,
        clawback_window_seconds: 0,
        current_epoch: 0,
        cumulative_obligated: 0,
        cumulative_claimed: 0,
        bump: dist_bump,
    };
    set_anchor_account(&mut svm, &distributor, &weft::ID, &dist);

    let esc = weft::PaymentEscrow {
        owner,
        mint,
        vault: escrow_vault,
        balance: DEPOSIT,
        total_deposited: DEPOSIT,
        total_spent: 0,
        bump: escrow_bump,
    };
    set_anchor_account(&mut svm, &escrow, &weft::ID, &esc);

    Env { svm, poster, mint, distributor, escrow, escrow_vault, reward_vault, treasury }
}

fn settle_ix(env: &Env, authority: &Pubkey, amount: u64) -> Instruction {
    Instruction {
        program_id: weft::ID,
        accounts: weft::accounts::SettleFromEscrow {
            settle_authority: *authority,
            distributor: env.distributor,
            escrow: env.escrow,
            escrow_vault: env.escrow_vault,
            reward_mint: env.mint,
            reward_vault: env.reward_vault,
            treasury: env.treasury,
            token_program: TOKEN_2022,
        }
        .to_account_metas(None),
        data: weft::instruction::SettleFromEscrow { amount }.data(),
    }
}

#[test]
fn settle_splits_70_10_burns_20_and_debits_escrow() {
    let mut env = setup();
    let supply_before = mint_supply(&env.svm, &env.mint);
    assert_eq!(supply_before, DEPOSIT);

    let ix = settle_ix(&env, &env.poster.pubkey(), DEPOSIT);
    let poster = env.poster.insecure_clone();
    send(&mut env.svm, &[ix], &poster, &[&poster]);

    // 70% node reward vault, 10% treasury, 20% burned.
    assert_eq!(token_amount(&env.svm, &env.reward_vault), 700_000, "reward vault = 70%");
    assert_eq!(token_amount(&env.svm, &env.treasury), 100_000, "treasury = 10%");
    assert_eq!(token_amount(&env.svm, &env.escrow_vault), 0, "escrow vault drained");
    assert_eq!(mint_supply(&env.svm, &env.mint), supply_before - 200_000, "20% burned");

    let (balance, spent) = escrow_balance(&env.svm, &env.escrow);
    assert_eq!(balance, 0, "escrow balance debited to zero");
    assert_eq!(spent, DEPOSIT, "total_spent recorded");
}

#[test]
fn partial_settle_leaves_remainder_withdrawable() {
    let mut env = setup();
    let ix = settle_ix(&env, &env.poster.pubkey(), 400_000);
    let poster = env.poster.insecure_clone();
    send(&mut env.svm, &[ix], &poster, &[&poster]);

    assert_eq!(token_amount(&env.svm, &env.reward_vault), 280_000, "70% of 400k");
    assert_eq!(token_amount(&env.svm, &env.treasury), 40_000, "10% of 400k");
    let (balance, spent) = escrow_balance(&env.svm, &env.escrow);
    assert_eq!(balance, 600_000, "unspent balance stays in escrow (owner-withdrawable)");
    assert_eq!(spent, 400_000);
}

#[test]
fn non_poster_cannot_settle() {
    let mut env = setup();
    let stranger = Keypair::new();
    env.svm.airdrop(&stranger.pubkey(), 1_000_000_000).unwrap();

    let ix = settle_ix(&env, &stranger.pubkey(), DEPOSIT);
    let msg = Message::new(&[ix], Some(&stranger.pubkey()));
    let tx = Transaction::new(&[&stranger], msg, env.svm.latest_blockhash());
    assert!(env.svm.send_transaction(tx).is_err(), "only poster_authority may settle");
    // Nothing moved.
    assert_eq!(token_amount(&env.svm, &env.reward_vault), 0);
    assert_eq!(escrow_balance(&env.svm, &env.escrow).0, DEPOSIT);
}

#[test]
fn cannot_settle_more_than_escrow_balance() {
    let mut env = setup();
    let ix = settle_ix(&env, &env.poster.pubkey(), DEPOSIT + 1);
    let poster = env.poster.insecure_clone();
    let msg = Message::new(&[ix], Some(&poster.pubkey()));
    let tx = Transaction::new(&[&poster], msg, env.svm.latest_blockhash());
    assert!(env.svm.send_transaction(tx).is_err(), "overdraw must be rejected");
    assert_eq!(token_amount(&env.svm, &env.reward_vault), 0);
    assert_eq!(escrow_balance(&env.svm, &env.escrow).0, DEPOSIT);
}
