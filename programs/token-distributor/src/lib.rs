// False positives from the Anchor `#[program]` macro expansion, not our code.
#![allow(clippy::diverging_sub_expression)]

pub mod constants;
pub mod error;
pub mod external;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("89wHYTG6deQUn4Rsbkcw2gG2sVSk8g9jTezP9qxG66Dn");

#[program]
pub mod token_distributor {
    use super::*;

    /// Create the IDO distributor + its vault (governance funds the vault separately).
    pub fn initialize_ido(
        ctx: Context<InitializeIdo>,
        merkle_root: [u8; 32],
        tge_ts: i64,
        tge_bps: u32,
        vesting_duration: i64,
        total_allocation: u64,
    ) -> Result<()> {
        ctx.accounts.initialize_ido(
            merkle_root,
            tge_ts,
            tge_bps,
            vesting_duration,
            total_allocation,
            ctx.bumps.distributor,
        )
    }

    /// Replace the allocation merkle root (governance, pre-TGE only).
    pub fn set_root(
        ctx: Context<SetRoot>,
        merkle_root: [u8; 32],
        total_allocation: u64,
    ) -> Result<()> {
        ctx.accounts.set_root(merkle_root, total_allocation)
    }

    /// Claim an IDO allocation: 25% liquid now + 75% into a 12-month vesting schedule.
    pub fn claim(ctx: Context<Claim>, amount: u64, proof: Vec<[u8; 32]>) -> Result<()> {
        ctx.accounts
            .claim(amount, proof, ctx.bumps.allocation_claim)
    }
}
