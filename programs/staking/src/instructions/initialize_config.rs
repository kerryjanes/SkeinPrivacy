use anchor_lang::prelude::*;
use anchor_spl::token_interface::Mint;

use crate::{constants::*, error::StakingError, state::StakingConfig};

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub mint: InterfaceAccount<'info, Mint>,
    /// CHECK: token account slashed stake flows to.
    pub treasury: UncheckedAccount<'info>,
    /// CHECK: slash authority (governance / settlement).
    pub slash_authority: UncheckedAccount<'info>,
    #[account(init, payer = authority, space = 8 + StakingConfig::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Account<'info, StakingConfig>,
    pub system_program: Program<'info, System>,
}

impl InitializeConfig<'_> {
    pub fn initialize_config(&mut self, unbonding_seconds: i64, bump: u8) -> Result<()> {
        require!(unbonding_seconds >= 0, StakingError::InvalidUnbonding);
        self.config.set_inner(StakingConfig {
            authority: self.authority.key(),
            slash_authority: self.slash_authority.key(),
            treasury: self.treasury.key(),
            mint: self.mint.key(),
            unbonding_seconds,
            bump,
        });
        Ok(())
    }
}
