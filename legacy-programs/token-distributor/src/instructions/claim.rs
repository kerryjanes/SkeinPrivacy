use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};
use weft_primitives::{
    merkle::{hash_allocation_leaf, merkle_verify},
    split_tge,
};

use crate::{
    constants::*,
    error::DistributorError,
    external::{invoke_create_schedule, WEFT_VESTING_ID},
    state::{AllocationClaim, IdoDistributor},
};

/// Schedule id for a claimant's single IDO vesting schedule.
const IDO_SCHEDULE_ID: u64 = 0;

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub claimant: Signer<'info>,
    #[account(
        mut,
        seeds = [IDO_SEED],
        bump = distributor.bump,
        has_one = mint,
        has_one = vault,
    )]
    pub distributor: Account<'info, IdoDistributor>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    /// The claimant's ATA: receives the full allocation, then funds the vesting CPI.
    #[account(mut, token::mint = mint, token::authority = claimant)]
    pub claimant_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(
        init,
        payer = claimant,
        space = 8 + AllocationClaim::INIT_SPACE,
        seeds = [ALLOC_SEED, claimant.key().as_ref()],
        bump
    )]
    pub allocation_claim: Account<'info, AllocationClaim>,

    // ---- weft-vesting CPI (creates the 75% schedule funded from the claimant ATA) ----
    /// CHECK: the weft-vesting program.
    #[account(address = WEFT_VESTING_ID)]
    pub vesting_program: UncheckedAccount<'info>,
    /// CHECK: the new VestingSchedule PDA (validated inside the CPI).
    #[account(mut)]
    pub schedule: UncheckedAccount<'info>,
    /// CHECK: the new schedule vault PDA (validated inside the CPI).
    #[account(mut)]
    pub schedule_vault: UncheckedAccount<'info>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl Claim<'_> {
    pub fn claim(&mut self, amount: u64, proof: Vec<[u8; 32]>, claim_bump: u8) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(now >= self.distributor.tge_ts, DistributorError::BeforeTge);

        // Verify the allocation against the posted root (distributor-bound leaf).
        let leaf = hash_allocation_leaf(
            &self.distributor.key().to_bytes(),
            &self.claimant.key().to_bytes(),
            amount,
        );
        require!(
            merkle_verify(&proof, self.distributor.merkle_root, leaf),
            DistributorError::InvalidProof
        );

        // Solvency + split.
        require!(
            self.vault.amount >= amount,
            DistributorError::InsufficientVault
        );
        let (tge, vest) = split_tge(amount, self.distributor.tge_bps);
        let new_claimed = self
            .distributor
            .total_claimed
            .checked_add(amount)
            .ok_or(DistributorError::MathOverflow)?;
        require!(
            new_claimed <= self.distributor.total_allocation,
            DistributorError::Overclaim
        );

        // Move the FULL allocation to the claimant ATA (signed by the distributor PDA);
        // the vesting CPI then pulls the 75% back out, leaving the 25% TGE liquid.
        let dist_bump = self.distributor.bump;
        let seeds: &[&[&[u8]]] = &[&[IDO_SEED, &[dist_bump]]];
        let dec = self.mint.decimals;
        transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.key(),
                TransferChecked {
                    from: self.vault.to_account_info(),
                    mint: self.mint.to_account_info(),
                    to: self.claimant_token_account.to_account_info(),
                    authority: self.distributor.to_account_info(),
                },
                seeds,
            ),
            amount,
            dec,
        )?;

        // Create the 75% linear schedule (cliff 0, start = TGE, non-revocable), funded by
        // the claimant from the ATA we just credited. The claimant is the funder/payer.
        if vest > 0 {
            invoke_create_schedule(
                &self.vesting_program.to_account_info(),
                &self.claimant.to_account_info(),
                &self.claimant.to_account_info(),
                &self.distributor.to_account_info(),
                &self.mint.to_account_info(),
                &self.schedule.to_account_info(),
                &self.schedule_vault.to_account_info(),
                &self.claimant_token_account.to_account_info(),
                &self.token_program.to_account_info(),
                &self.system_program.to_account_info(),
                IDO_SCHEDULE_ID,
                vest,
                0,
                self.distributor.tge_ts,
                0,
                self.distributor.vesting_duration,
                false,
            )?;
        }

        self.distributor.total_claimed = new_claimed;
        self.allocation_claim.set_inner(AllocationClaim {
            claimant: self.claimant.key(),
            amount,
            tge_amount: tge,
            vesting_amount: vest,
            claimed_at: now,
            bump: claim_bump,
        });
        Ok(())
    }
}
