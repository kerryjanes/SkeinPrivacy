use anchor_lang::prelude::*;

use crate::{constants::*, error::ReputationError, state::ReputationConfig};

#[derive(Accounts)]
pub struct SetOracle<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = config.authority == authority.key() @ ReputationError::Unauthorized,
    )]
    pub config: Account<'info, ReputationConfig>,
}

impl SetOracle<'_> {
    /// Re-point the metrics oracle (e.g. to the rewards-settlement authority PDA in M4).
    pub fn set_oracle(&mut self, new_oracle: Pubkey) -> Result<()> {
        self.config.oracle = new_oracle;
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
        constraint = config.authority == authority.key() @ ReputationError::Unauthorized,
    )]
    pub config: Account<'info, ReputationConfig>,
}

impl TransferAuthority<'_> {
    /// Hand the top-level config authority to a new key — used during M5 bootstrap to
    /// transfer control to the governance authority PDA.
    pub fn transfer_authority(&mut self, new_authority: Pubkey) -> Result<()> {
        self.config.authority = new_authority;
        Ok(())
    }
}
