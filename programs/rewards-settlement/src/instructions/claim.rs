use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};
use weft_primitives::merkle::{hash_reward_leaf, merkle_verify};

use crate::{
    constants::*,
    error::SettlementError,
    state::{ClaimStatus, Distributor, EpochDistribution},
};

#[derive(Accounts)]
#[instruction(epoch: u64, node_id: u64)]
pub struct Claim<'info> {
    #[account(mut)]
    pub claimant: Signer<'info>,
    #[account(mut, seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, has_one = reward_mint, has_one = reward_vault)]
    pub distributor: Account<'info, Distributor>,
    #[account(mut, seeds = [EPOCH_SEED, &epoch.to_le_bytes()], bump = epoch_distribution.bump)]
    pub epoch_distribution: Account<'info, EpochDistribution>,
    /// CHECK: the leaf's operator (reward recipient); bound into the leaf hash.
    pub operator: UncheckedAccount<'info>,
    #[account(
        init,
        payer = claimant,
        space = 8 + ClaimStatus::INIT_SPACE,
        seeds = [CLAIM_SEED, &epoch.to_le_bytes(), operator.key().as_ref(), &node_id.to_le_bytes()],
        bump
    )]
    pub claim_status: Account<'info, ClaimStatus>,
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, token::mint = reward_mint, token::authority = operator)]
    pub operator_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl Claim<'_> {
    pub fn claim(
        &mut self,
        epoch: u64,
        node_id: u64,
        amount: u64,
        proof: Vec<[u8; 32]>,
        claim_bump: u8,
    ) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= self.epoch_distribution.posted_at + self.distributor.dispute_window_seconds,
            SettlementError::DisputeWindowOpen
        );

        let leaf = hash_reward_leaf(epoch, &self.operator.key().to_bytes(), node_id, amount);
        require!(
            merkle_verify(&proof, self.epoch_distribution.merkle_root, leaf),
            SettlementError::InvalidProof
        );

        // Effects before interaction.
        self.epoch_distribution.total_claimed = self
            .epoch_distribution
            .total_claimed
            .checked_add(amount)
            .ok_or(SettlementError::MathOverflow)?;
        require!(
            self.epoch_distribution.total_claimed <= self.epoch_distribution.total_reward,
            SettlementError::EpochOverclaim
        );
        self.distributor.cumulative_claimed = self
            .distributor
            .cumulative_claimed
            .checked_add(amount)
            .ok_or(SettlementError::MathOverflow)?;
        self.claim_status.set_inner(ClaimStatus {
            epoch,
            operator: self.operator.key(),
            node_id,
            amount,
            claimed_at: now,
            disputed: false,
            bump: claim_bump,
        });

        // Pay from the vault, signed by the distributor PDA.
        let distributor_bump = self.distributor.bump;
        let seeds: &[&[&[u8]]] = &[&[DISTRIBUTOR_SEED, &[distributor_bump]]];
        let dec = self.reward_mint.decimals;
        transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.key(),
                TransferChecked {
                    from: self.reward_vault.to_account_info(),
                    mint: self.reward_mint.to_account_info(),
                    to: self.operator_token_account.to_account_info(),
                    authority: self.distributor.to_account_info(),
                },
                seeds,
            ),
            amount,
            dec,
        )
    }
}
