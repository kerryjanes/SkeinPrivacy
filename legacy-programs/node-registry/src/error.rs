use anchor_lang::prelude::*;

#[error_code]
pub enum RegistryError {
    #[msg("Caller is not the registry authority")]
    Unauthorized,
    #[msg("Registry is paused")]
    Paused,
    #[msg("Invalid geo value")]
    InvalidGeo,
    #[msg("Invalid capability flags")]
    InvalidCapabilities,
    #[msg("Invalid availability value")]
    InvalidAvailability,
    #[msg("Invalid tree parameters")]
    InvalidTree,
    #[msg("Tree shard index must be sequential")]
    TreeIndexMismatch,
    #[msg("Active tree is full")]
    TreeFull,
    #[msg("Node is not active")]
    NodeNotActive,
    #[msg("Arithmetic overflow")]
    MathOverflow,
}
