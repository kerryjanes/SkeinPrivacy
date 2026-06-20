use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::RegistryError,
    state::{NodeState, Registry},
};

#[derive(Accounts)]
pub struct SetStake<'info> {
    /// The staking program's `[b"authority"]` signer PDA (passed via CPI).
    pub staking_authority: Signer<'info>,

    #[account(
        seeds = [REGISTRY_SEED],
        bump = registry.bump,
        constraint = registry.staking_authority == staking_authority.key() @ RegistryError::Unauthorized,
    )]
    pub registry: Account<'info, Registry>,

    #[account(
        mut,
        seeds = [NODE_SEED, node.operator.as_ref(), &node.node_id.to_le_bytes()],
        bump = node.bump,
    )]
    pub node: Account<'info, NodeState>,
}

impl SetStake<'_> {
    pub fn set_stake(&mut self, amount: u64) -> Result<()> {
        self.node.stake_amount = amount;
        self.node.updated_at = Clock::get()?.unix_timestamp;
        Ok(())
    }
}
