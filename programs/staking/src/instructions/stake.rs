use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{
    constants::*,
    error::StakingError,
    external::NODE_REGISTRY_ID,
    mirror::mirror_stake,
    state::{StakePosition, StakingConfig},
};

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct Stake<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, StakingConfig>,
    #[account(
        init_if_needed,
        payer = operator,
        space = 8 + StakePosition::INIT_SPACE,
        seeds = [STAKE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump
    )]
    pub position: Account<'info, StakePosition>,
    #[account(
        init_if_needed,
        payer = operator,
        token::mint = mint,
        token::authority = position,
        seeds = [VAULT_SEED, position.key().as_ref()],
        bump
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, token::mint = mint, token::authority = operator)]
    pub operator_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(address = config.mint)]
    pub mint: InterfaceAccount<'info, Mint>,
    /// CHECK: program signer PDA for the mirror CPI.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub program_authority: UncheckedAccount<'info>,
    /// CHECK: node-registry program.
    #[account(address = NODE_REGISTRY_ID)]
    pub node_registry_program: UncheckedAccount<'info>,
    /// CHECK: node-registry Registry PDA (validated by node-registry).
    pub registry: UncheckedAccount<'info>,
    /// CHECK: NodeState PDA, optional; validated by node-registry when present.
    #[account(mut)]
    pub node: Option<UncheckedAccount<'info>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl Stake<'_> {
    pub fn stake(
        &mut self,
        node_id: u64,
        amount: u64,
        lock_duration: i64,
        position_bump: u8,
        authority_bump: u8,
    ) -> Result<()> {
        require!(amount > 0, StakingError::ZeroAmount);
        require!(
            (MIN_LOCK_SECONDS..=MAX_LOCK_SECONDS).contains(&lock_duration),
            StakingError::InvalidLock
        );
        let now = Clock::get()?.unix_timestamp;

        self.position.operator = self.operator.key();
        self.position.node_id = node_id;
        self.position.mint = self.mint.key();
        self.position.vault = self.vault.key();
        self.position.bump = position_bump;
        self.position.amount = self
            .position
            .amount
            .checked_add(amount)
            .ok_or(StakingError::MathOverflow)?;
        let new_lock = now
            .checked_add(lock_duration)
            .ok_or(StakingError::MathOverflow)?;
        if new_lock > self.position.locked_until {
            self.position.locked_until = new_lock;
        }

        let decimals = self.mint.decimals;
        let cpi = CpiContext::new(
            self.token_program.key(),
            TransferChecked {
                from: self.operator_token_account.to_account_info(),
                mint: self.mint.to_account_info(),
                to: self.vault.to_account_info(),
                authority: self.operator.to_account_info(),
            },
        );
        transfer_checked(cpi, amount, decimals)?;

        let nrp = self.node_registry_program.to_account_info();
        let pa = self.program_authority.to_account_info();
        let reg = self.registry.to_account_info();
        let node_ai = self.node.as_ref().map(|n| n.to_account_info());
        mirror_stake(
            &nrp,
            &pa,
            &reg,
            node_ai.as_ref(),
            authority_bump,
            self.position.amount,
        )
    }
}
