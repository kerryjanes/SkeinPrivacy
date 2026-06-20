use anchor_lang::prelude::*;

/// Singleton reputation config. PDA `[CONFIG_SEED]`.
#[account]
#[derive(InitSpace)]
pub struct ReputationConfig {
    pub authority: Pubkey,
    /// Permissioned submitter of uptime/speed/review measurements (M5 → governance).
    pub oracle: Pubkey,
    pub bump: u8,
}

/// One node's reputation. PDA `[STATE_SEED, operator, node_id_le]`.
#[account]
#[derive(InitSpace)]
pub struct ReputationState {
    pub operator: Pubkey,
    pub node_id: u64,
    /// EMA-smoothed quality score (0..=BPS); drives the multiplier.
    pub quality_bps: u32,
    /// Derived reward multiplier (5000..=20000), mirrored into `NodeState.reputation`.
    pub multiplier_bps: u32,
    /// Last raw sample (for the indexer / debugging).
    pub uptime_bps: u32,
    pub speed_bps: u32,
    pub review_bps: u32,
    pub updated_at: i64,
    pub bump: u8,
}
