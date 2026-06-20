// False positives from the Anchor `#[program]` macro expansion, not our code.
#![allow(clippy::diverging_sub_expression)]

pub mod constants;
pub mod error;
pub mod instructions;
pub mod mirror;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("6Nwa73bqP56LNQwWEKJWAp4A5RJKSMBzFxdxtuq3Y86u");

#[program]
pub mod reputation {
    use super::*;

    pub fn initialize_config(ctx: Context<InitializeConfig>) -> Result<()> {
        ctx.accounts.initialize_config(ctx.bumps.config)
    }

    pub fn update_metrics(
        ctx: Context<UpdateMetrics>,
        node_id: u64,
        uptime_bps: u32,
        speed_bps: u32,
        review_bps: u32,
    ) -> Result<()> {
        ctx.accounts.update_metrics(
            node_id,
            uptime_bps,
            speed_bps,
            review_bps,
            ctx.bumps.state,
            ctx.bumps.program_authority,
        )
    }

    pub fn penalize(ctx: Context<Penalize>, severity_bps: u32) -> Result<()> {
        ctx.accounts
            .penalize(severity_bps, ctx.bumps.program_authority)
    }

    pub fn resync(ctx: Context<Resync>) -> Result<()> {
        ctx.accounts.resync(ctx.bumps.program_authority)
    }
}
