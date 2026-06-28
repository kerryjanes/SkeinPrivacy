use anchor_lang::prelude::*;

/// Singleton staking config. PDA `[CONFIG_SEED]`.
#[account]
#[derive(InitSpace)]
pub struct StakingConfig {
    pub authority: Pubkey,
    /// Who may slash positions (governance / settlement, M4/M5).
    pub slash_authority: Pubkey,
    /// Token account slashed stake flows to (pinned; not chosen at slash time).
    pub treasury: Pubkey,
    /// $WEFT mint.
    pub mint: Pubkey,
    /// Seconds a `request_unstake` must wait before `withdraw_unstaked`.
    pub unbonding_seconds: i64,
    pub bump: u8,
}

/// One node's stake. PDA `[STAKE_SEED, operator, node_id_le]`.
#[account]
#[derive(InitSpace)]
pub struct StakePosition {
    pub operator: Pubkey,
    pub node_id: u64,
    pub mint: Pubkey,
    pub vault: Pubkey,
    /// Total bonded (includes any amount currently unbonding).
    pub amount: u64,
    /// Amount requested for withdrawal, releasable at `unbonding_until`.
    pub unbonding_amount: u64,
    /// No unstake requests allowed before this time.
    pub locked_until: i64,
    /// Requested unstake becomes withdrawable at this time.
    pub unbonding_until: i64,
    pub bump: u8,
}
