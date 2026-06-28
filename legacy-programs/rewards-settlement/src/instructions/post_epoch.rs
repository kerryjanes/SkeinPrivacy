use anchor_lang::prelude::*;
use anchor_spl::token_interface::TokenAccount;

use crate::{
    constants::*,
    error::SettlementError,
    state::{Distributor, EpochDistribution},
};

#[derive(Accounts)]
#[instruction(epoch: u64)]
pub struct PostEpoch<'info> {
    #[account(mut)]
    pub poster: Signer<'info>,
    #[account(
        mut,
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        has_one = reward_vault,
        constraint = distributor.poster_authority == poster.key() @ SettlementError::Unauthorized,
    )]
    pub distributor: Account<'info, Distributor>,
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(
        init,
        payer = poster,
        space = 8 + EpochDistribution::INIT_SPACE,
        seeds = [EPOCH_SEED, &epoch.to_le_bytes()],
        bump
    )]
    pub epoch_distribution: Account<'info, EpochDistribution>,
    pub system_program: Program<'info, System>,
}

impl PostEpoch<'_> {
    pub fn post_epoch(
        &mut self,
        epoch: u64,
        merkle_root: [u8; 32],
        total_reward: u64,
        num_nodes: u32,
        bump: u8,
    ) -> Result<()> {
        require!(
            epoch > self.distributor.current_epoch,
            SettlementError::NonMonotonicEpoch
        );

        // Solvency: free vault balance must cover already-outstanding + this epoch.
        let outstanding = self
            .distributor
            .cumulative_obligated
            .checked_sub(self.distributor.cumulative_claimed)
            .ok_or(SettlementError::MathOverflow)?;
        let needed = outstanding
            .checked_add(total_reward)
            .ok_or(SettlementError::MathOverflow)?;
        require!(
            self.reward_vault.amount >= needed,
            SettlementError::InsufficientVault
        );

        let now = Clock::get()?.unix_timestamp;
        self.epoch_distribution.set_inner(EpochDistribution {
            epoch,
            merkle_root,
            total_reward,
            total_claimed: 0,
            num_nodes,
            posted_at: now,
            swept: false,
            bump,
        });
        self.distributor.cumulative_obligated = self
            .distributor
            .cumulative_obligated
            .checked_add(total_reward)
            .ok_or(SettlementError::MathOverflow)?;
        self.distributor.current_epoch = epoch;
        Ok(())
    }
}
