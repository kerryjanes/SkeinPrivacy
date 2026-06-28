pub const GOVERNANCE_CONFIG_SEED: &[u8] = b"governance_config";
pub const PROPOSAL_SEED: &[u8] = b"proposal";
pub const PROPOSAL_TX_SEED: &[u8] = b"proposal_tx";
pub const VOTE_SEED: &[u8] = b"vote";
pub const PROTOCOL_CONFIG_SEED: &[u8] = b"protocol_config";
/// Program-signer PDA that holds every governed authority + the DAO treasury.
pub const GOVERNANCE_AUTHORITY_SEED: &[u8] = b"governance_authority";

/// Caps on a single queued instruction (multiple `ProposalTransaction`s batch more).
pub const MAX_TX_ACCOUNTS: usize = 24;
pub const MAX_TX_DATA: usize = 1024;
/// Maximum proposal name length (bytes).
pub const MAX_PROPOSAL_NAME: usize = 64;
