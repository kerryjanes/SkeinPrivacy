use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{
    constants::*,
    error::StakingError,
    mirror::mirror_stake,
    state::{StakePosition, StakingConfig},
};

#[derive(Accounts)]
pub struct Slash<'info> {
    pub slash_authority: Signer<'info>,
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = config.slash_authority == slash_authority.key() @ StakingError::Unauthorized,
        has_one = treasury,
    )]
    pub config: Account<'info, StakingConfig>,
    #[account(
        mut,
        seeds = [STAKE_SEED, position.operator.as_ref(), &position.node_id.to_le_bytes()],
        bump = position.bump,
        has_one = vault,
    )]
    pub position: Account<'info, StakePosition>,
    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, address = config.treasury)]
    pub treasury: InterfaceAccount<'info, TokenAccount>,
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

impl Slash<'_> {
    pub fn slash(&mut self, amount: u64, authority_bump: u8) -> Result<()> {
        let slashed = amount.min(self.position.amount);
        require!(slashed > 0, StakingError::ZeroAmount);

        let operator = self.position.operator;
        let id = self.position.node_id.to_le_bytes();
        let bump = self.position.bump;
        let seeds: &[&[&[u8]]] = &[&[STAKE_SEED, operator.as_ref(), &id, &[bump]]];
        let decimals = self.mint.decimals;
        let cpi = CpiContext::new_with_signer(
            self.token_program.key(),
            TransferChecked {
                from: self.vault.to_account_info(),
                mint: self.mint.to_account_info(),
                to: self.treasury.to_account_info(),
                authority: self.position.to_account_info(),
            },
            seeds,
        );
        transfer_checked(cpi, slashed, decimals)?;

        self.position.amount = self.position.amount.saturating_sub(slashed);
        if self.position.unbonding_amount > self.position.amount {
            self.position.unbonding_amount = self.position.amount;
        }

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
