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

declare_id!("BMQZKvCbq8qcZFWWGt1S7ZXg8odZcMbUA3oaNKnQi7mz");

#[program]
pub mod rewards_settlement {
    use super::*;

    pub fn initialize_distributor(
        ctx: Context<InitializeDistributor>,
        dispute_window_seconds: i64,
        clawback_window_seconds: i64,
    ) -> Result<()> {
        ctx.accounts.initialize_distributor(
            dispute_window_seconds,
            clawback_window_seconds,
            ctx.bumps.distributor,
        )
    }

    pub fn set_authorities(
        ctx: Context<SetAuthorities>,
        poster_authority: Pubkey,
        dispute_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts
            .set_authorities(poster_authority, dispute_authority)
    }

    /// User pays for traffic: 70% → reward vault, 20% burned, 10% → treasury.
    pub fn pay_traffic(ctx: Context<PayTraffic>, amount: u64) -> Result<()> {
        ctx.accounts.pay_traffic(amount)
    }

    /// Emission top-up of the reward vault.
    pub fn fund_vault(ctx: Context<FundVault>, amount: u64) -> Result<()> {
        ctx.accounts.fund_vault(amount)
    }

    /// Aggregator posts an epoch's reward merkle root.
    pub fn post_epoch(
        ctx: Context<PostEpoch>,
        epoch: u64,
        merkle_root: [u8; 32],
        total_reward: u64,
        num_nodes: u32,
    ) -> Result<()> {
        ctx.accounts.post_epoch(
            epoch,
            merkle_root,
            total_reward,
            num_nodes,
            ctx.bumps.epoch_distribution,
        )
    }

    /// A node claims its epoch reward with a merkle proof.
    pub fn claim(
        ctx: Context<Claim>,
        epoch: u64,
        node_id: u64,
        amount: u64,
        proof: Vec<[u8; 32]>,
    ) -> Result<()> {
        ctx.accounts
            .claim(epoch, node_id, amount, proof, ctx.bumps.claim_status)
    }

    /// Reclaim an epoch's unclaimed residual after the clawback window.
    pub fn sweep_epoch(ctx: Context<SweepEpoch>, _epoch: u64) -> Result<()> {
        ctx.accounts.sweep_epoch()
    }
}
