use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::SettlementError,
    state::{Distributor, EpochDistribution},
};

#[derive(Accounts)]
#[instruction(epoch: u64)]
pub struct SweepEpoch<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        constraint = distributor.authority == authority.key() @ SettlementError::Unauthorized,
    )]
    pub distributor: Account<'info, Distributor>,
    #[account(mut, seeds = [EPOCH_SEED, &epoch.to_le_bytes()], bump = epoch_distribution.bump)]
    pub epoch_distribution: Account<'info, EpochDistribution>,
}

impl SweepEpoch<'_> {
    /// After the clawback window, de-obligate an epoch's unclaimed residual.
    pub fn sweep_epoch(&mut self) -> Result<()> {
        require!(
            !self.epoch_distribution.swept,
            SettlementError::AlreadySwept
        );
        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= self.epoch_distribution.posted_at + self.distributor.clawback_window_seconds,
            SettlementError::ClawbackWindowOpen
        );
        let residual = self
            .epoch_distribution
            .total_reward
            .checked_sub(self.epoch_distribution.total_claimed)
            .ok_or(SettlementError::MathOverflow)?;
        self.distributor.cumulative_obligated = self
            .distributor
            .cumulative_obligated
            .checked_sub(residual)
            .ok_or(SettlementError::MathOverflow)?;
        self.epoch_distribution.swept = true;
        Ok(())
    }
}
