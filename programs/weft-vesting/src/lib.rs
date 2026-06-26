// False positives from the Anchor `#[program]` macro expansion, not our code.
// `too_many_arguments` fires on the generated `cpi::create_schedule` wrapper (a 7-param
// vesting instruction) once another program enables this crate's `cpi` feature.
#![allow(clippy::diverging_sub_expression, clippy::too_many_arguments)]

pub mod constants;
pub mod error;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("Aa8aMMVmxcA5CKQAJQ3N3EmFtKYTx79hEEYjFYzBCDjb");

#[program]
pub mod weft_vesting {
    use super::*;

    /// Create a vesting schedule and atomically fund its vault with `total_amount`
    /// tokens pulled from `funder_token_account`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_schedule(
        ctx: Context<CreateSchedule>,
        schedule_id: u64,
        total_amount: u64,
        cliff_unlock_amount: u64,
        start_ts: i64,
        cliff_duration: i64,
        duration: i64,
        revocable: bool,
    ) -> Result<()> {
        ctx.accounts.create_schedule(
            schedule_id,
            total_amount,
            cliff_unlock_amount,
            start_ts,
            cliff_duration,
            duration,
            revocable,
            ctx.bumps.schedule,
        )
    }

    /// Release vested-but-unclaimed tokens to a beneficiary-chosen destination.
    pub fn claim(ctx: Context<Claim>) -> Result<()> {
        ctx.accounts.claim()
    }

    /// Revoke a revocable schedule: freeze accrual and reclaim the unvested
    /// remainder to the authority. The beneficiary may still claim what vested
    /// up to the revocation time.
    pub fn revoke(ctx: Context<Revoke>) -> Result<()> {
        ctx.accounts.revoke()
    }
}
