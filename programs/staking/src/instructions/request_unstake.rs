use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::StakingError,
    state::{StakePosition, StakingConfig},
};

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct RequestUnstake<'info> {
    pub operator: Signer<'info>,
    #[account(
        mut,
        seeds = [STAKE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump = position.bump,
        constraint = position.operator == operator.key() @ StakingError::Unauthorized,
    )]
    pub position: Account<'info, StakePosition>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, StakingConfig>,
}

impl RequestUnstake<'_> {
    pub fn request_unstake(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, StakingError::ZeroAmount);
        let now = Clock::get()?.unix_timestamp;
        require!(now >= self.position.locked_until, StakingError::Locked);
        let available = self
            .position
            .amount
            .checked_sub(self.position.unbonding_amount)
            .ok_or(StakingError::MathOverflow)?;
        require!(amount <= available, StakingError::InsufficientStake);
        self.position.unbonding_amount = self
            .position
            .unbonding_amount
            .checked_add(amount)
            .ok_or(StakingError::MathOverflow)?;
        self.position.unbonding_until = now
            .checked_add(self.config.unbonding_seconds)
            .ok_or(StakingError::MathOverflow)?;
        Ok(())
    }
}
