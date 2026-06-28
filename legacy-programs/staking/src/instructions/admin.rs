use anchor_lang::prelude::*;

use crate::{constants::*, error::StakingError, state::StakingConfig};

#[derive(Accounts)]
pub struct SetSlashAuthority<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = config.authority == authority.key() @ StakingError::Unauthorized,
    )]
    pub config: Account<'info, StakingConfig>,
}

impl SetSlashAuthority<'_> {
    /// Re-point who may slash (e.g. to the rewards-settlement authority PDA in M4).
    pub fn set_slash_authority(&mut self, new_slash_authority: Pubkey) -> Result<()> {
        self.config.slash_authority = new_slash_authority;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct TransferAuthority<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = config.authority == authority.key() @ StakingError::Unauthorized,
    )]
    pub config: Account<'info, StakingConfig>,
}

impl TransferAuthority<'_> {
    /// Hand the top-level config authority to a new key — used during M5 bootstrap to
    /// transfer control to the governance authority PDA.
    pub fn transfer_authority(&mut self, new_authority: Pubkey) -> Result<()> {
        self.config.authority = new_authority;
        Ok(())
    }
}
