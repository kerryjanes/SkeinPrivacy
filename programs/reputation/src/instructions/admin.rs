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
