use anchor_lang::prelude::*;

#[error_code]
pub enum ReputationError {
    #[msg("Caller is not the reputation oracle")]
    Unauthorized,
    #[msg("Metric value out of range")]
    InvalidMetric,
}
