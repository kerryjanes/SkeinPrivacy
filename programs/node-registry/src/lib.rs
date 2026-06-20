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

    /// Register a node: mint its Bubblegum V2 cNFT and create its `NodeState`.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        ctx: Context<Register>,
        node_id: u64,
        geo: u32,
        capabilities: u32,
        endpoint_hash: [u8; 32],
        availability: u8,
        metadata_uri: String,
    ) -> Result<()> {
        ctx.accounts.register(
            node_id,
            geo,
            capabilities,
            endpoint_hash,
            availability,
            metadata_uri,
            ctx.bumps.node,
        )
    }

    /// Update a node's mutable on-chain state (operator only).
    pub fn update(
        ctx: Context<UpdateNode>,
        geo: Option<u32>,
        capabilities: Option<u32>,
        endpoint_hash: Option<[u8; 32]>,
        availability: Option<u8>,
    ) -> Result<()> {
        ctx.accounts
            .update(geo, capabilities, endpoint_hash, availability)
    }

    /// Deregister a node: remove its authoritative `NodeState` record (rent →
    /// operator). The operator keeps the cNFT and may burn it independently.
    pub fn deregister(ctx: Context<Deregister>) -> Result<()> {
        ctx.accounts.deregister()
    }

    /// Mirror a node's staked balance into `NodeState` (staking program only).
    pub fn set_stake(ctx: Context<SetStake>, amount: u64) -> Result<()> {
        ctx.accounts.set_stake(amount)
    }

    /// Mirror a node's reputation multiplier into `NodeState` (reputation program only).
    pub fn set_reputation(ctx: Context<SetReputation>, reputation_bps: u16) -> Result<()> {
        ctx.accounts.set_reputation(reputation_bps)
    }

    /// Rotate the registry authority.
    pub fn set_authority(ctx: Context<AdminRegistry>, new_authority: Pubkey) -> Result<()> {
        ctx.accounts.set_authority(new_authority)
    }

    /// Pause or unpause node registration.
    pub fn set_paused(ctx: Context<AdminRegistry>, paused: bool) -> Result<()> {
        ctx.accounts.set_paused(paused)
    }

    /// Set the staking/reputation metric-writer authorities (admin).
    pub fn set_metrics_authorities(
        ctx: Context<AdminRegistry>,
        reputation_authority: Pubkey,
        staking_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts
            .set_metrics_authorities(reputation_authority, staking_authority)
    }
}
