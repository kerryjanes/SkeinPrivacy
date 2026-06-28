use anchor_lang::prelude::*;

use crate::{constants::*, error::RegistryError, state::Registry};

#[derive(Accounts)]
pub struct AdminRegistry<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [REGISTRY_SEED],
        bump = registry.bump,
        has_one = authority @ RegistryError::Unauthorized,
    )]
    pub registry: Account<'info, Registry>,
}

impl AdminRegistry<'_> {
    pub fn set_authority(&mut self, new_authority: Pubkey) -> Result<()> {
        self.registry.authority = new_authority;
        Ok(())
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<()> {
        self.registry.paused = paused;
        Ok(())
    }

    /// Point the gated metric writers at the staking/reputation programs'
    /// `[b"authority"]` signer PDAs (only they may then mirror into `NodeState`).
    pub fn set_metrics_authorities(
        &mut self,
        reputation_authority: Pubkey,
        staking_authority: Pubkey,
    ) -> Result<()> {
        self.registry.reputation_authority = reputation_authority;
        self.registry.staking_authority = staking_authority;
        Ok(())
    }
}
