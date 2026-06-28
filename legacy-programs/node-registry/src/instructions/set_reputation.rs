use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::RegistryError,
    state::{NodeState, Registry},
};

#[derive(Accounts)]
pub struct SetReputation<'info> {
    /// The reputation program's `[b"authority"]` signer PDA (passed via CPI).
    pub reputation_authority: Signer<'info>,

    #[account(
        seeds = [REGISTRY_SEED],
        bump = registry.bump,
        constraint = registry.reputation_authority == reputation_authority.key() @ RegistryError::Unauthorized,
    )]
    pub registry: Account<'info, Registry>,

    #[account(
        mut,
        seeds = [NODE_SEED, node.operator.as_ref(), &node.node_id.to_le_bytes()],
        bump = node.bump,
    )]
    pub node: Account<'info, NodeState>,
}

impl SetReputation<'_> {
    /// `reputation_bps` is the reward multiplier (5000..=20000) computed off the
    /// reputation program's quality score.
    pub fn set_reputation(&mut self, reputation_bps: u16) -> Result<()> {
        self.node.reputation = reputation_bps;
        self.node.updated_at = Clock::get()?.unix_timestamp;
        Ok(())
    }
}
