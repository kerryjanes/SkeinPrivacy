// False positives from the Anchor `#[program]` macro expansion, not our code.
#![allow(clippy::diverging_sub_expression)]

pub mod constants;
pub mod error;
pub mod external;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("3Xn3H6DBCVhJxz2kGSBEJj8tKYVfyLzJ1ugG8VHViJe5");

#[program]
pub mod rewards_settlement {
    use super::*;

    pub fn initialize_distributor(
        ctx: Context<InitializeDistributor>,
        dispute_window_seconds: i64,
        clawback_window_seconds: i64,
    ) -> Result<()> {
        ctx.accounts.initialize_distributor(
            dispute_window_seconds,
            clawback_window_seconds,
            ctx.bumps.distributor,
        )
    }

    pub fn set_authorities(
        ctx: Context<SetAuthorities>,
        poster_authority: Pubkey,
        dispute_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts
            .set_authorities(poster_authority, dispute_authority)
    }

    /// Transfer the distributor authority (M5: hand control to the governance PDA).
    pub fn transfer_authority(
        ctx: Context<TransferAuthority>,
        new_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts.transfer_authority(new_authority)
    }

    /// User pays for traffic: 70% → reward vault, 20% burned, 10% → treasury.
    pub fn pay_traffic(ctx: Context<PayTraffic>, amount: u64) -> Result<()> {
        ctx.accounts.pay_traffic(amount)
    }

    /// Prepay $WEFT into the user's traffic escrow.
    pub fn deposit_escrow(ctx: Context<DepositEscrow>, amount: u64) -> Result<()> {
        ctx.accounts.deposit_escrow(amount, ctx.bumps.escrow)
    }

    /// Settle traffic from prepaid escrow with the same split/burn as `pay_traffic`.
    pub fn pay_traffic_from_escrow(ctx: Context<PayTrafficFromEscrow>, amount: u64) -> Result<()> {
        ctx.accounts.pay_traffic_from_escrow(amount)
    }

    /// Withdraw unused prepaid $WEFT from the user's traffic escrow.
    pub fn withdraw_escrow(ctx: Context<WithdrawEscrow>, amount: u64) -> Result<()> {
        ctx.accounts.withdraw_escrow(amount)
    }

    /// Emission top-up of the reward vault.
    pub fn fund_vault(ctx: Context<FundVault>, amount: u64) -> Result<()> {
        ctx.accounts.fund_vault(amount)
    }

    /// Aggregator posts an epoch's reward merkle root.
    pub fn post_epoch(
        ctx: Context<PostEpoch>,
        epoch: u64,
        merkle_root: [u8; 32],
        total_reward: u64,
        num_nodes: u32,
    ) -> Result<()> {
        ctx.accounts.post_epoch(
            epoch,
            merkle_root,
            total_reward,
            num_nodes,
            ctx.bumps.epoch_distribution,
        )
    }

    /// A node claims its epoch reward with a merkle proof.
    pub fn claim(
        ctx: Context<Claim>,
        epoch: u64,
        node_id: u64,
        amount: u64,
        proof: Vec<[u8; 32]>,
    ) -> Result<()> {
        ctx.accounts
            .claim(epoch, node_id, amount, proof, ctx.bumps.claim_status)
    }

    /// Reclaim an epoch's unclaimed residual after the clawback window.
    pub fn sweep_epoch(ctx: Context<SweepEpoch>, _epoch: u64) -> Result<()> {
        ctx.accounts.sweep_epoch()
    }

    /// Flag a fraudulent leaf: block the payout and slash + penalize the node.
    pub fn dispute(
        ctx: Context<Dispute>,
        epoch: u64,
        node_id: u64,
        amount: u64,
        severity_bps: u32,
        slash_amount: u64,
    ) -> Result<()> {
        ctx.accounts.dispute(
            epoch,
            node_id,
            amount,
            severity_bps,
            slash_amount,
            ctx.bumps.claim_status,
            ctx.bumps.program_authority,
        )
    }
}
