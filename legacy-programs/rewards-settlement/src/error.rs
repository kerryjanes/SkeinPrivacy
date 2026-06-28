use anchor_lang::prelude::*;

#[error_code]
pub enum SettlementError {
    #[msg("Caller is not authorized")]
    Unauthorized,
    #[msg("Epoch must be strictly increasing")]
    NonMonotonicEpoch,
    #[msg("Reward vault cannot cover the posted obligations")]
    InsufficientVault,
    #[msg("Merkle proof is invalid")]
    InvalidProof,
    #[msg("Dispute window has not elapsed")]
    DisputeWindowOpen,
    #[msg("This leaf has been disputed")]
    Disputed,
    #[msg("Epoch over-claimed")]
    EpochOverclaim,
    #[msg("Clawback window has not elapsed")]
    ClawbackWindowOpen,
    #[msg("Epoch already swept")]
    AlreadySwept,
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Escrow balance is insufficient")]
    InsufficientEscrow,
    #[msg("Escrow account does not match the expected owner, mint, or vault")]
    InvalidEscrow,
}
