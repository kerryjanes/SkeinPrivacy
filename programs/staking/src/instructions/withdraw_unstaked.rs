use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{constants::*, error::StakingError, mirror::mirror_stake, state::StakePosition};

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct WithdrawUnstaked<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,
    #[account(
        mut,
        seeds = [STAKE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump = position.bump,
        constraint = position.operator == operator.key() @ StakingError::Unauthorized,
        has_one = vault,
    )]
    pub position: Account<'info, StakePosition>,
    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, token::mint = mint, token::authority = operator)]
    pub operator_token_account: InterfaceAccount<'info, TokenAccount>,
    pub mint: InterfaceAccount<'info, Mint>,
    /// CHECK: program signer PDA for the mirror CPI.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub program_authority: UncheckedAccount<'info>,
    /// CHECK: node-registry program.
    #[account(address = node_registry::ID)]
    pub node_registry_program: UncheckedAccount<'info>,
    /// CHECK: node-registry Registry PDA.
    pub registry: UncheckedAccount<'info>,
    /// CHECK: NodeState PDA, optional.
    #[account(mut)]
    pub node: Option<UncheckedAccount<'info>>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl WithdrawUnstaked<'_> {
    pub fn withdraw_unstaked(&mut self, node_id: u64, authority_bump: u8) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let amount = self.position.unbonding_amount;
        require!(amount > 0, StakingError::ZeroAmount);
        require!(
            now >= self.position.unbonding_until,
            StakingError::StillUnbonding
        );

        let operator = self.position.operator;
        let id = node_id.to_le_bytes();
        let bump = self.position.bump;
        let seeds: &[&[&[u8]]] = &[&[STAKE_SEED, operator.as_ref(), &id, &[bump]]];
        let decimals = self.mint.decimals;
        let cpi = CpiContext::new_with_signer(
            self.token_program.key(),
            TransferChecked {
                from: self.vault.to_account_info(),
                mint: self.mint.to_account_info(),
                to: self.operator_token_account.to_account_info(),
                authority: self.position.to_account_info(),
            },
            seeds,
        );
        transfer_checked(cpi, amount, decimals)?;

        self.position.amount = self
            .position
            .amount
            .checked_sub(amount)
            .ok_or(StakingError::MathOverflow)?;
        self.position.unbonding_amount = 0;

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
