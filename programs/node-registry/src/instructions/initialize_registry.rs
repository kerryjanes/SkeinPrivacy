use anchor_lang::prelude::*;

use crate::{constants::*, state::Registry};

#[derive(Accounts)]
pub struct InitializeRegistry<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: the MPL-Core collection these node cNFTs belong to (provisioned off-chain).
    pub collection: UncheckedAccount<'info>,

    /// CHECK: the initial active merkle tree (provisioned off-chain).
    pub active_tree: UncheckedAccount<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + Registry::INIT_SPACE,
        seeds = [REGISTRY_SEED],
        bump
    )]
    pub registry: Account<'info, Registry>,

    pub system_program: Program<'info, System>,
}

impl InitializeRegistry<'_> {
    pub fn initialize_registry(&mut self, bump: u8) -> Result<()> {
        let authority = self.authority.key();
        self.registry.set_inner(Registry {
            authority,
            collection: self.collection.key(),
            active_tree: self.active_tree.key(),
            tree_count: 0,
            node_count: 0,
            reputation_authority: authority,
            staking_authority: authority,
            paused: false,
            bump,
        });
        Ok(())
    }
}
