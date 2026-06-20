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

declare_id!("86FwTDBau7T289G9Fnkjn34g7NN3furoGEDwFsLVXzTK");

#[program]
pub mod staking {
    use super::*;

    pub fn initialize_config(ctx: Context<InitializeConfig>, unbonding_seconds: i64) -> Result<()> {
        ctx.accounts
            .initialize_config(unbonding_seconds, ctx.bumps.config)
    }

    pub fn stake(ctx: Context<Stake>, node_id: u64, amount: u64, lock_duration: i64) -> Result<()> {
        ctx.accounts.stake(
            node_id,
            amount,
            lock_duration,
            ctx.bumps.position,
            ctx.bumps.program_authority,
        )
    }

    pub fn request_unstake(ctx: Context<RequestUnstake>, node_id: u64, amount: u64) -> Result<()> {
        let _ = node_id;
        ctx.accounts.request_unstake(amount)
    }

    pub fn withdraw_unstaked(ctx: Context<WithdrawUnstaked>, node_id: u64) -> Result<()> {
        ctx.accounts
            .withdraw_unstaked(node_id, ctx.bumps.program_authority)
    }

    pub fn slash(ctx: Context<Slash>, amount: u64) -> Result<()> {
        ctx.accounts.slash(amount, ctx.bumps.program_authority)
    }

    pub fn resync(ctx: Context<Resync>) -> Result<()> {
        ctx.accounts.resync(ctx.bumps.program_authority)
    }

    pub fn set_slash_authority(
        ctx: Context<SetSlashAuthority>,
        new_slash_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts.set_slash_authority(new_slash_authority)
    }

    /// Transfer the config authority (M5: hand control to the governance PDA).
    pub fn transfer_authority(
        ctx: Context<TransferAuthority>,
        new_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts.transfer_authority(new_authority)
    }
}
