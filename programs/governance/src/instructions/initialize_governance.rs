use anchor_lang::prelude::*;

use crate::{constants::*, error::GovError, state::GovernanceConfig};

#[derive(Accounts)]
pub struct InitializeGovernance<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    /// CHECK: the $WEFT governance mint (recorded for reference).
    pub gov_mint: UncheckedAccount<'info>,
    #[account(
        init,
        payer = authority,
        space = 8 + GovernanceConfig::INIT_SPACE,
        seeds = [GOVERNANCE_CONFIG_SEED],
        bump
    )]
    pub governance_config: Account<'info, GovernanceConfig>,
    pub system_program: Program<'info, System>,
}

impl InitializeGovernance<'_> {
    pub fn initialize_governance(
        &mut self,
        default_quorum: u64,
        default_approval_threshold_bps: u32,
        voting_period_seconds: i64,
        execution_delay_seconds: i64,
        min_proposal_stake: u64,
        bump: u8,
    ) -> Result<()> {
        require!(
            default_approval_threshold_bps <= weft_primitives::BPS
                && voting_period_seconds > 0
                && execution_delay_seconds >= 0,
            GovError::InvalidParam
        );
        self.governance_config.set_inner(GovernanceConfig {
            authority: self.authority.key(),
            gov_mint: self.gov_mint.key(),
            default_quorum,
            default_approval_threshold_bps,
            voting_period_seconds,
            execution_delay_seconds,
            min_proposal_stake,
            proposal_count: 0,
            bump,
        });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct SetGovernanceAuthority<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [GOVERNANCE_CONFIG_SEED],
        bump = governance_config.bump,
        constraint = governance_config.authority == authority.key() @ GovError::Unauthorized,
    )]
    pub governance_config: Account<'info, GovernanceConfig>,
}

impl SetGovernanceAuthority<'_> {
    pub fn set_governance_authority(&mut self, new_authority: Pubkey) -> Result<()> {
        self.governance_config.authority = new_authority;
        Ok(())
    }
}
