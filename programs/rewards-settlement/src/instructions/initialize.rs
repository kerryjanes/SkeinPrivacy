use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::{constants::*, error::SettlementError, state::Distributor};

#[derive(Accounts)]
pub struct InitializeDistributor<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub reward_mint: InterfaceAccount<'info, Mint>,
    /// CHECK: the off-chain aggregator (poster).
    pub poster_authority: UncheckedAccount<'info>,
    /// CHECK: the dispute authority.
    pub dispute_authority: UncheckedAccount<'info>,
    /// CHECK: treasury token account (10% of payments).
    pub treasury: UncheckedAccount<'info>,
    #[account(init, payer = authority, space = 8 + Distributor::INIT_SPACE, seeds = [DISTRIBUTOR_SEED], bump)]
    pub distributor: Account<'info, Distributor>,
    #[account(
        init,
        payer = authority,
        token::mint = reward_mint,
        token::authority = distributor,
        seeds = [VAULT_SEED],
        bump
    )]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl InitializeDistributor<'_> {
    pub fn initialize_distributor(
        &mut self,
        dispute_window_seconds: i64,
        clawback_window_seconds: i64,
        bump: u8,
    ) -> Result<()> {
        require!(
            dispute_window_seconds >= 0 && clawback_window_seconds >= 0,
            SettlementError::ZeroAmount
        );
        self.distributor.set_inner(Distributor {
            authority: self.authority.key(),
            poster_authority: self.poster_authority.key(),
            dispute_authority: self.dispute_authority.key(),
            reward_mint: self.reward_mint.key(),
            reward_vault: self.reward_vault.key(),
            treasury: self.treasury.key(),
            dispute_window_seconds,
            clawback_window_seconds,
            current_epoch: 0,
            cumulative_obligated: 0,
            cumulative_claimed: 0,
            bump,
        });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct SetAuthorities<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        constraint = distributor.authority == authority.key() @ SettlementError::Unauthorized,
    )]
    pub distributor: Account<'info, Distributor>,
}

impl SetAuthorities<'_> {
    pub fn set_authorities(
        &mut self,
        poster_authority: Pubkey,
        dispute_authority: Pubkey,
    ) -> Result<()> {
        self.distributor.poster_authority = poster_authority;
        self.distributor.dispute_authority = dispute_authority;
        Ok(())
    }
}
