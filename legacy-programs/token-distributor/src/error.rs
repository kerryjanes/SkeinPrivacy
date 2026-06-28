use anchor_lang::prelude::*;

#[error_code]
pub enum DistributorError {
    #[msg("Caller is not the distributor authority")]
    Unauthorized,
    #[msg("Merkle proof did not verify against the posted root")]
    InvalidProof,
    #[msg("Claiming is not open until the TGE timestamp")]
    BeforeTge,
    #[msg("The root may only be changed before the TGE")]
    RootLockedAfterTge,
    #[msg("Claim would exceed the total allocation")]
    Overclaim,
    #[msg("Distributor vault has insufficient balance")]
    InsufficientVault,
    #[msg("Parameter out of range")]
    InvalidParam,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
