use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::RegistryError,
    state::{Registry, TreeShard},
};

#[derive(Accounts)]
#[instruction(index: u16)]
pub struct RegisterTree<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [REGISTRY_SEED],
        bump = registry.bump,
        has_one = authority @ RegistryError::Unauthorized,
    )]
    pub registry: Account<'info, Registry>,

    /// CHECK: the merkle tree account (provisioned off-chain, delegated to the registry PDA).
    pub merkle_tree: UncheckedAccount<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + TreeShard::INIT_SPACE,
        seeds = [TREE_SEED, &index.to_le_bytes()],
        bump
    )]
    pub tree_shard: Account<'info, TreeShard>,

    pub system_program: Program<'info, System>,
}

impl RegisterTree<'_> {
    pub fn register_tree(&mut self, index: u16, max_depth: u32, bump: u8) -> Result<()> {
        require!(
            index == self.registry.tree_count,
            RegistryError::TreeIndexMismatch
        );
        require!(
            max_depth > 0 && max_depth <= MAX_TREE_DEPTH,
            RegistryError::InvalidTree
        );

        self.tree_shard.set_inner(TreeShard {
            merkle_tree: self.merkle_tree.key(),
            index,
            minted: 0,
            capacity: 1u64 << max_depth,
            full: false,
            bump,
        });

        self.registry.active_tree = self.merkle_tree.key();
        self.registry.tree_count = self
            .registry
            .tree_count
            .checked_add(1)
            .ok_or(RegistryError::MathOverflow)?;
        Ok(())
    }
}
