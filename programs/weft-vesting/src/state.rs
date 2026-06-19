use anchor_lang::prelude::*;

/// A single cliff + linear vesting schedule, funding one PDA-owned vault.
#[account]
#[derive(InitSpace)]
pub struct VestingSchedule {
    /// Who may `claim` released tokens.
    pub beneficiary: Pubkey,
    /// Who may `revoke` (and receives the reclaimed unvested remainder).
    pub authority: Pubkey,
    /// The $WEFT mint this schedule vests.
    pub mint: Pubkey,
    /// The PDA-owned vault holding not-yet-claimed tokens.
    pub vault: Pubkey,
    /// Disambiguator allowing multiple schedules per beneficiary (PDA seed).
    pub schedule_id: u64,
    /// Total tokens deposited at creation (base units). Immutable.
    pub total_amount: u64,
    /// Amount unlocked immediately at `start_ts` (TGE lump). 0 for all M1 schedules.
    pub cliff_unlock_amount: u64,
    /// Cumulative amount already transferred to the beneficiary.
    pub released_amount: u64,
    /// Unix timestamp vesting is measured from (TGE / genesis).
    pub start_ts: i64,
    /// Seconds after `start_ts` before any linear amount unlocks.
    pub cliff_duration: i64,
    /// Total seconds from `start_ts` to fully vested.
    pub duration: i64,
    /// Whether `authority` may revoke and reclaim unvested tokens.
    pub revocable: bool,
    /// Set once revoked; freezes further accrual.
    pub revoked: bool,
    /// Timestamp accrual was frozen at (valid only when `revoked`).
    pub revoked_ts: i64,
    /// PDA bump for signing vault CPIs.
    pub bump: u8,
}

impl VestingSchedule {
    /// Effective "now" for vesting math — frozen at revocation if revoked.
    pub fn effective_now(&self, clock_now: i64) -> i64 {
        if self.revoked {
            self.revoked_ts
        } else {
            clock_now
        }
    }

    /// Cumulative vested amount at `clock_now`, accounting for revocation.
    pub fn vested(&self, clock_now: i64) -> u64 {
        weft_primitives::vested_amount(
            self.total_amount,
            self.cliff_unlock_amount,
            self.start_ts,
            self.cliff_duration,
            self.duration,
            self.effective_now(clock_now),
        )
    }
}
