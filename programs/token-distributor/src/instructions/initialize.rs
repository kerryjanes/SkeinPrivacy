use anchor_lang::prelude::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

use crate::{constants::*, error::DistributorError, state::IdoDistributor};

#[derive(Accounts)]
pub struct InitializeIdo<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(
        init,
        payer = authority,
        space = 8 + IdoDistributor::INIT_SPACE,
        seeds = [IDO_SEED],
        bump
    )]
    pub distributor: Account<'info, IdoDistributor>,
    #[account(
        init,
        payer = authority,
        token::mint = mint,
        token::authority = distributor,
        seeds = [b"ido_vault"],
        bump
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl InitializeIdo<'_> {
    pub fn initialize_ido(
        &mut self,
        merkle_root: [u8; 32],
        tge_ts: i64,
        tge_bps: u32,
        vesting_duration: i64,
        total_allocation: u64,
        bump: u8,
    ) -> Result<()> {
        require!(
            tge_bps <= weft_primitives::BPS && vesting_duration > 0,
            DistributorError::InvalidParam
        );
        self.distributor.set_inner(IdoDistributor {
            authority: self.authority.key(),
            mint: self.mint.key(),
            vault: self.vault.key(),
            merkle_root,
            tge_ts,
            tge_bps,
            vesting_duration,
            total_allocation,
            total_claimed: 0,
            bump,
        });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct SetRoot<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [IDO_SEED],
        bump = distributor.bump,
        constraint = distributor.authority == authority.key() @ DistributorError::Unauthorized,
    )]
    pub distributor: Account<'info, IdoDistributor>,
}

impl SetRoot<'_> {
    /// Replace the allocation root — only before the TGE (post-TGE it is immutable so
    /// claimants can rely on it).
    pub fn set_root(&mut self, merkle_root: [u8; 32], total_allocation: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(
            now < self.distributor.tge_ts,
            DistributorError::RootLockedAfterTge
        );
        self.distributor.merkle_root = merkle_root;
        self.distributor.total_allocation = total_allocation;
        Ok(())
    }
}
