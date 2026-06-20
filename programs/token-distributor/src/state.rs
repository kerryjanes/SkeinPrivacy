use anchor_lang::prelude::*;

/// The IDO distribution singleton. PDA `[IDO_SEED]`. Holds the merkle root governance
/// posts, the vault funding all claims, and the TGE/vesting parameters.
#[account]
#[derive(InitSpace)]
pub struct IdoDistributor {
    /// Governance authority (the DAO PDA after bootstrap) — may set the root pre-TGE.
    pub authority: Pubkey,
    pub mint: Pubkey,
    /// PDA token account holding the full IDO allocation (TGE + vesting pool).
    pub vault: Pubkey,
    pub merkle_root: [u8; 32],
    /// Unix timestamp claiming opens.
    pub tge_ts: i64,
    /// Share released immediately at TGE (bps); the rest vests linearly.
    pub tge_bps: u32,
    /// Linear vesting duration for the non-TGE portion (seconds).
    pub vesting_duration: i64,
    /// Total allocation across the whole tree (for the solvency invariant).
    pub total_allocation: u64,
    /// Σ claimed so far.
    pub total_claimed: u64,
    pub bump: u8,
}

/// One claimant's claim marker. PDA `[ALLOC_SEED, claimant]`. `init` makes a claimant
/// claimable exactly once (atomic double-claim guard).
#[account]
#[derive(InitSpace)]
pub struct AllocationClaim {
    pub claimant: Pubkey,
    pub amount: u64,
    pub tge_amount: u64,
    pub vesting_amount: u64,
    pub claimed_at: i64,
    pub bump: u8,
}
