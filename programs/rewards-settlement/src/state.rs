use anchor_lang::prelude::*;

/// Singleton settlement config. PDA `[DISTRIBUTOR_SEED]`.
#[account]
#[derive(InitSpace)]
pub struct Distributor {
    pub authority: Pubkey,
    /// The off-chain aggregator that posts epoch roots.
    pub poster_authority: Pubkey,
    /// Who may flag fraudulent leaves (→ slash + reputation penalty).
    pub dispute_authority: Pubkey,
    pub reward_mint: Pubkey,
    /// PDA token account = the node-reward pool (emissions + 70% of user payments).
    pub reward_vault: Pubkey,
    pub treasury: Pubkey,
    pub dispute_window_seconds: i64,
    pub clawback_window_seconds: i64,
    pub current_epoch: u64,
    /// Σ total_reward posted across all epochs.
    pub cumulative_obligated: u64,
    /// Σ claimed across all epochs.
    pub cumulative_claimed: u64,
    pub bump: u8,
}

/// One epoch's reward root. PDA `[EPOCH_SEED, epoch_le]`.
#[account]
#[derive(InitSpace)]
pub struct EpochDistribution {
    pub epoch: u64,
    pub merkle_root: [u8; 32],
    pub total_reward: u64,
    pub total_claimed: u64,
    pub num_nodes: u32,
    pub posted_at: i64,
    pub swept: bool,
    pub bump: u8,
}

/// Per-(epoch, operator, node) claim/dispute marker. PDA `[CLAIM_SEED, epoch_le, operator, node_id_le]`.
/// Created by either `claim` (paid) or `dispute` (blocked) — `init` makes the two mutually exclusive.
#[account]
#[derive(InitSpace)]
pub struct ClaimStatus {
    pub epoch: u64,
    pub operator: Pubkey,
    pub node_id: u64,
    pub amount: u64,
    pub claimed_at: i64,
    pub disputed: bool,
    pub bump: u8,
}

/// Per-wallet prepaid traffic balance. PDA `[ESCROW_SEED, owner]`.
#[account]
#[derive(InitSpace)]
pub struct PaymentEscrow {
    pub owner: Pubkey,
    pub mint: Pubkey,
    /// PDA token account owned by this escrow account.
    pub vault: Pubkey,
    /// Unspent prepaid balance still available for VPN traffic.
    pub balance: u64,
    pub total_deposited: u64,
    pub total_spent: u64,
    pub bump: u8,
}
