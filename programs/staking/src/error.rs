use anchor_lang::prelude::*;

#[error_code]
pub enum StakingError {
    #[msg("Caller is not authorized")]
    Unauthorized,
    #[msg("Stake is still locked")]
    Locked,
    #[msg("Unbonding window has not elapsed")]
    StillUnbonding,
    #[msg("Invalid lock duration")]
    InvalidLock,
    #[msg("Invalid unbonding duration")]
    InvalidUnbonding,
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Insufficient staked balance")]
    InsufficientStake,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
