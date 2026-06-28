use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::RegistryError,
    state::{NodeState, Registry},
};

#[derive(Accounts)]
pub struct Deregister<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,

    #[account(mut, seeds = [REGISTRY_SEED], bump = registry.bump)]
    pub registry: Account<'info, Registry>,

    #[account(
        mut,
        seeds = [NODE_SEED, operator.key().as_ref(), &node.node_id.to_le_bytes()],
        bump = node.bump,
        has_one = operator @ RegistryError::Unauthorized,
        close = operator,
    )]
    pub node: Account<'info, NodeState>,
}

impl Deregister<'_> {
    pub fn deregister(&mut self) -> Result<()> {
        // Remove the authoritative registry record (rent → operator). The cNFT
        // remains owned by the operator (leaf_owner), who may burn it
        // independently via Bubblegum `burnV2` with a merkle proof.
        self.registry.node_count = self.registry.node_count.saturating_sub(1);
        Ok(())
    }
}
