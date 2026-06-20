pub const CONFIG_SEED: &[u8] = b"staking_config";
pub const STAKE_SEED: &[u8] = b"stake";
pub const VAULT_SEED: &[u8] = b"vault";
/// Program signer PDA seed for the node-registry mirror CPI.
pub const AUTHORITY_SEED: &[u8] = b"authority";

pub const MIN_LOCK_SECONDS: i64 = 0;
pub const MAX_LOCK_SECONDS: i64 = 4 * 365 * 24 * 3_600; // 4 years
