//! Multi-program LiteSVM tests: load node-registry + staking, prove the
//! cross-program stake mirror into NodeState plus lock/unbond/slash.

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
    solana_program_option::COption,
    solana_program_pack::Pack,
    solana_pubkey::Pubkey,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
    spl_token_interface::state::{Account as TokenState, AccountState, Mint as MintState},
};

const NODE_REGISTRY_BYTES: &[u8] = include_bytes!("../../../target/deploy/node_registry.so");
const STAKING_BYTES: &[u8] = include_bytes!("../../../target/deploy/staking.so");
const DECIMALS: u8 = 9;

fn token_program_id() -> Pubkey {
    spl_token_interface::ID
}

fn setup() -> (LiteSVM, Keypair) {
    let mut svm = LiteSVM::new();
    svm.add_program(node_registry::ID, NODE_REGISTRY_BYTES)
        .unwrap();
    svm.add_program(staking::ID, STAKING_BYTES).unwrap();
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
    Pubkey::find_program_address(&[staking::CONFIG_SEED], &staking::ID).0
}
fn position_pda(op: &Pubkey, id: u64) -> Pubkey {
    Pubkey::find_program_address(
        &[staking::STAKE_SEED, op.as_ref(), &id.to_le_bytes()],
        &staking::ID,
    )
    .0
}
fn vault_pda(position: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[staking::VAULT_SEED, position.as_ref()], &staking::ID).0
}
fn staking_authority_pda() -> Pubkey {
    Pubkey::find_program_address(&[staking::AUTHORITY_SEED], &staking::ID).0
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
            owner: token_program_id(),
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

    // point staking_authority at the staking program's [b"authority"] PDA
    let data = node_registry::instruction::SetMetricsAuthorities {
        reputation_authority: Pubkey::new_unique(),
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
fn node_stake(svm: &LiteSVM, node: &Pubkey) -> u64 {
    node_registry::NodeState::try_deserialize(&mut svm.get_account(node).unwrap().data.as_slice())
        .unwrap()
        .stake_amount
}

// ---- staking setup + ix builders ----
fn init_config(
    svm: &mut LiteSVM,
    authority: &Keypair,
    mint: &Pubkey,
    treasury: &Pubkey,
    slash_authority: &Pubkey,
    unbonding: i64,
) {
    let data = staking::instruction::InitializeConfig {
        unbonding_seconds: unbonding,
    }
    .data();
    let metas = staking::accounts::InitializeConfig {
        authority: authority.pubkey(),
        mint: *mint,
        treasury: *treasury,
        slash_authority: *slash_authority,
        config: config_pda(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    send(
        svm,
        Instruction::new_with_bytes(staking::ID, &data, metas),
        authority,
        &[authority],
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn stake_ix(
    operator: &Pubkey,
    mint: &Pubkey,
    op_ta: &Pubkey,
    node_id: u64,
    amount: u64,
    lock: i64,
    node: Option<Pubkey>,
) -> Instruction {
    let position = position_pda(operator, node_id);
    let data = staking::instruction::Stake {
        node_id,
        amount,
        lock_duration: lock,
    }
    .data();
    let metas = staking::accounts::Stake {
        operator: *operator,
        config: config_pda(),
        position,
        vault: vault_pda(&position),
        operator_token_account: *op_ta,
        mint: *mint,
        program_authority: staking_authority_pda(),
        node_registry_program: node_registry::ID,
        registry: registry_pda(),
        node,
        token_program: token_program_id(),
        system_program: anchor_lang::solana_program::system_program::ID,
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(staking::ID, &data, metas)
}

fn get_position(svm: &LiteSVM, op: &Pubkey, id: u64) -> staking::StakePosition {
    staking::StakePosition::try_deserialize(
        &mut svm
            .get_account(&position_pda(op, id))
            .unwrap()
            .data
            .as_slice(),
    )
    .unwrap()
}

#[test]
fn stake_mirrors_into_node_state() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, 1_000);
    init_registry(&mut svm, &authority);
    let mint = create_mint(&mut svm);
    let treasury = create_token_account(&mut svm, &mint, &Pubkey::new_unique(), 0);
    init_config(
        &mut svm,
        &authority,
        &mint,
        &treasury,
        &authority.pubkey(),
        100,
    );

    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 1_000_000_000_000).unwrap();
    let node = craft_node(&mut svm, &op.pubkey(), 1);
    let op_ta = create_token_account(&mut svm, &mint, &op.pubkey(), 50_000);

    send(
        &mut svm,
        stake_ix(&op.pubkey(), &mint, &op_ta, 1, 30_000, 0, Some(node)),
        &op,
        &[&op],
    )
    .unwrap();

    assert_eq!(get_position(&svm, &op.pubkey(), 1).amount, 30_000);
    assert_eq!(
        token_amount(&svm, &vault_pda(&position_pda(&op.pubkey(), 1))),
        30_000
    );
    assert_eq!(token_amount(&svm, &op_ta), 20_000);
    assert_eq!(node_stake(&svm, &node), 30_000); // mirrored!
}

#[test]
fn stake_without_node_state_succeeds() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, 1_000);
    init_registry(&mut svm, &authority);
    let mint = create_mint(&mut svm);
    let treasury = create_token_account(&mut svm, &mint, &Pubkey::new_unique(), 0);
    init_config(
        &mut svm,
        &authority,
        &mint,
        &treasury,
        &authority.pubkey(),
        100,
    );

    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 1_000_000_000_000).unwrap();
    let op_ta = create_token_account(&mut svm, &mint, &op.pubkey(), 50_000);

    // no node account → mirror is a no-op, stake still works
    send(
        &mut svm,
        stake_ix(&op.pubkey(), &mint, &op_ta, 7, 10_000, 0, None),
        &op,
        &[&op],
    )
    .unwrap();
    assert_eq!(get_position(&svm, &op.pubkey(), 7).amount, 10_000);
}

#[test]
fn restake_extends_lock_never_shortens() {
    let (mut svm, authority) = setup();
    set_clock(&mut svm, 1_000);
    init_registry(&mut svm, &authority);
    let mint = create_mint(&mut svm);
    let treasury = create_token_account(&mut svm, &mint, &Pubkey::new_unique(), 0);
    init_config(
        &mut svm,
        &authority,
        &mint,
        &treasury,
        &authority.pubkey(),
        100,
    );
    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 1_000_000_000_000).unwrap();
    let op_ta = create_token_account(&mut svm, &mint, &op.pubkey(), 50_000);

    send(
        &mut svm,
        stake_ix(&op.pubkey(), &mint, &op_ta, 1, 5_000, 5_000, None),
        &op,
        &[&op],
    )
    .unwrap();
    let locked = get_position(&svm, &op.pubkey(), 1).locked_until;
    assert_eq!(locked, 6_000);
    // re-stake with a shorter lock must not shorten
    send(
        &mut svm,
        stake_ix(&op.pubkey(), &mint, &op_ta, 1, 100, 100, None),
        &op,
        &[&op],
    )
    .unwrap();
    assert_eq!(get_position(&svm, &op.pubkey(), 1).locked_until, 6_000);
    assert_eq!(get_position(&svm, &op.pubkey(), 1).amount, 5_100);
}

fn request_unstake_ix(operator: &Pubkey, node_id: u64, amount: u64) -> Instruction {
    let data = staking::instruction::RequestUnstake { node_id, amount }.data();
    let metas = staking::accounts::RequestUnstake {
        operator: *operator,
        position: position_pda(operator, node_id),
        config: config_pda(),
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(staking::ID, &data, metas)
}

fn withdraw_ix(
    operator: &Pubkey,
    mint: &Pubkey,
    op_ta: &Pubkey,
    node_id: u64,
    node: Option<Pubkey>,
) -> Instruction {
    let position = position_pda(operator, node_id);
    let data = staking::instruction::WithdrawUnstaked { node_id }.data();
    let metas = staking::accounts::WithdrawUnstaked {
        operator: *operator,
        position,
        vault: vault_pda(&position),
        operator_token_account: *op_ta,
        mint: *mint,
        program_authority: staking_authority_pda(),
        node_registry_program: node_registry::ID,
        registry: registry_pda(),
        node,
        token_program: token_program_id(),
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(staking::ID, &data, metas)
}

#[allow(clippy::too_many_arguments)]
fn slash_ix(
    slasher: &Pubkey,
    operator: &Pubkey,
    mint: &Pubkey,
    treasury: &Pubkey,
    node_id: u64,
    amount: u64,
    node: Option<Pubkey>,
) -> Instruction {
    let position = position_pda(operator, node_id);
    let data = staking::instruction::Slash { amount }.data();
    let metas = staking::accounts::Slash {
        slash_authority: *slasher,
        config: config_pda(),
        position,
        vault: vault_pda(&position),
        treasury: *treasury,
        mint: *mint,
        program_authority: staking_authority_pda(),
        node_registry_program: node_registry::ID,
        registry: registry_pda(),
        node,
        token_program: token_program_id(),
    }
    .to_account_metas(None);
    Instruction::new_with_bytes(staking::ID, &data, metas)
}

fn full_setup(svm: &mut LiteSVM, authority: &Keypair, unbonding: i64) -> Pubkey {
    set_clock(svm, 1_000);
    init_registry(svm, authority);
    let mint = create_mint(svm);
    let treasury = create_token_account(svm, &mint, &Pubkey::new_unique(), 0);
    init_config(
        svm,
        authority,
        &mint,
        &treasury,
        &authority.pubkey(),
        unbonding,
    );
    mint
}

#[test]
fn unbonding_flow_gates_and_mirrors() {
    let (mut svm, authority) = setup();
    let mint = full_setup(&mut svm, &authority, 100);
    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 1_000_000_000_000).unwrap();
    let node = craft_node(&mut svm, &op.pubkey(), 1);
    let op_ta = create_token_account(&mut svm, &mint, &op.pubkey(), 50_000);
    send(
        &mut svm,
        stake_ix(&op.pubkey(), &mint, &op_ta, 1, 30_000, 50, Some(node)),
        &op,
        &[&op],
    )
    .unwrap();

    // request before lock expiry (1000 < 1050)
    assert_failed_with(
        send(
            &mut svm,
            request_unstake_ix(&op.pubkey(), 1, 10_000),
            &op,
            &[&op],
        ),
        "Locked",
    );
    set_clock(&mut svm, 1_100);
    send(
        &mut svm,
        request_unstake_ix(&op.pubkey(), 1, 10_000),
        &op,
        &[&op],
    )
    .unwrap();
    assert_eq!(get_position(&svm, &op.pubkey(), 1).unbonding_amount, 10_000);

    // withdraw before unbonding window (1100 < 1200)
    assert_failed_with(
        send(
            &mut svm,
            withdraw_ix(&op.pubkey(), &mint, &op_ta, 1, Some(node)),
            &op,
            &[&op],
        ),
        "StillUnbonding",
    );
    set_clock(&mut svm, 1_300);
    send(
        &mut svm,
        withdraw_ix(&op.pubkey(), &mint, &op_ta, 1, Some(node)),
        &op,
        &[&op],
    )
    .unwrap();
    assert_eq!(get_position(&svm, &op.pubkey(), 1).amount, 20_000);
    assert_eq!(token_amount(&svm, &op_ta), 30_000); // 20k staked left in vault
    assert_eq!(node_stake(&svm, &node), 20_000); // mirror decremented
}

#[test]
fn slash_to_treasury_saturates_and_gated() {
    let (mut svm, authority) = setup();
    let mint = full_setup(&mut svm, &authority, 100);
    let treasury = staking::StakingConfig::try_deserialize(
        &mut svm.get_account(&config_pda()).unwrap().data.as_slice(),
    )
    .unwrap()
    .treasury;
    let op = Keypair::new();
    svm.airdrop(&op.pubkey(), 1_000_000_000_000).unwrap();
    let node = craft_node(&mut svm, &op.pubkey(), 1);
    let op_ta = create_token_account(&mut svm, &mint, &op.pubkey(), 50_000);
    send(
        &mut svm,
        stake_ix(&op.pubkey(), &mint, &op_ta, 1, 30_000, 0, Some(node)),
        &op,
        &[&op],
    )
    .unwrap();

    // non-authority cannot slash
    let attacker = Keypair::new();
    svm.airdrop(&attacker.pubkey(), 1_000_000_000_000).unwrap();
    assert_failed_with(
        send(
            &mut svm,
            slash_ix(
                &attacker.pubkey(),
                &op.pubkey(),
                &mint,
                &treasury,
                1,
                5_000,
                Some(node),
            ),
            &attacker,
            &[&attacker],
        ),
        "Unauthorized",
    );

    // slash by authority, saturating (50k > 30k staked → 30k)
    send(
        &mut svm,
        slash_ix(
            &authority.pubkey(),
            &op.pubkey(),
            &mint,
            &treasury,
            1,
            50_000,
            Some(node),
        ),
        &authority,
        &[&authority],
    )
    .unwrap();
    assert_eq!(token_amount(&svm, &treasury), 30_000);
    assert_eq!(get_position(&svm, &op.pubkey(), 1).amount, 0);
    assert_eq!(node_stake(&svm, &node), 0); // mirror zeroed
}
