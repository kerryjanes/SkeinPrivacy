// False positives from the Anchor `#[program]` macro expansion, not our code.
#![allow(clippy::diverging_sub_expression)]

pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("6dsqVjMmczosqNk2kaFHa33ut9ZUAwazgUagPKk5tUgd");

#[program]
pub mod node_registry {
    use super::*;

    /// Initialize the singleton registry (collection + first active tree).
    pub fn initialize_registry(ctx: Context<InitializeRegistry>) -> Result<()> {
        ctx.accounts.initialize_registry(ctx.bumps.registry)
    }

    /// Provision a new merkle-tree shard and make it the active tree.
    pub fn register_tree(ctx: Context<RegisterTree>, index: u16, max_depth: u32) -> Result<()> {
        ctx.accounts
            .register_tree(index, max_depth, ctx.bumps.tree_shard)
    }

    /// Rotate the registry authority.
    pub fn set_authority(ctx: Context<AdminRegistry>, new_authority: Pubkey) -> Result<()> {
        ctx.accounts.set_authority(new_authority)
    }

    /// Pause or unpause node registration.
    pub fn set_paused(ctx: Context<AdminRegistry>, paused: bool) -> Result<()> {
        ctx.accounts.set_paused(paused)
    }
}
