use anchor_lang::prelude::*;

use crate::{constants::*, state::ReputationConfig};

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    /// CHECK: the reputation oracle (permissioned measurement submitter).
    pub oracle: UncheckedAccount<'info>,
    #[account(init, payer = authority, space = 8 + ReputationConfig::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Account<'info, ReputationConfig>,
    pub system_program: Program<'info, System>,
}

impl InitializeConfig<'_> {
    pub fn initialize_config(&mut self, bump: u8) -> Result<()> {
        self.config.set_inner(ReputationConfig {
            authority: self.authority.key(),
            oracle: self.oracle.key(),
            bump,
        });
        Ok(())
    }
}
