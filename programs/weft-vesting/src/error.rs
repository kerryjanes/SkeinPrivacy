use anchor_lang::prelude::*;

#[error_code]
pub enum VestingError {
    #[msg("Caller is not authorized for this schedule")]
    Unauthorized,
    #[msg("Nothing is currently claimable")]
    NothingToClaim,
    #[msg("Schedule is not revocable")]
    NotRevocable,
    #[msg("Schedule already revoked")]
    AlreadyRevoked,
    #[msg("Invalid schedule parameters")]
    InvalidParams,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
