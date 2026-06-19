use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{constants::*, error::VestingError, state::VestingSchedule};

#[derive(Accounts)]
pub struct Claim<'info> {
    pub beneficiary: Signer<'info>,

    #[account(
        mut,
        seeds = [SCHEDULE_SEED, beneficiary.key().as_ref(), &schedule.schedule_id.to_le_bytes()],
        bump = schedule.bump,
        has_one = beneficiary @ VestingError::Unauthorized,
        has_one = mint,
        has_one = vault,
    )]
    pub schedule: Account<'info, VestingSchedule>,

    pub mint: InterfaceAccount<'info, Mint>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    /// Beneficiary-chosen destination for the released tokens.
    #[account(mut, token::mint = mint)]
    pub destination: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

impl<'info> Claim<'info> {
    pub fn claim(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;

        let vested = self.schedule.vested(now);
        let releasable = vested
            .checked_sub(self.schedule.released_amount)
            .ok_or(VestingError::MathOverflow)?;
        require!(releasable > 0, VestingError::NothingToClaim);

        // Effects before interactions: record the release before transferring.
        self.schedule.released_amount = self
            .schedule
            .released_amount
            .checked_add(releasable)
            .ok_or(VestingError::MathOverflow)?;

        let beneficiary = self.schedule.beneficiary;
        let id_bytes = self.schedule.schedule_id.to_le_bytes();
        let bump = self.schedule.bump;
        let signer_seeds: &[&[&[u8]]] =
            &[&[SCHEDULE_SEED, beneficiary.as_ref(), &id_bytes, &[bump]]];

        let decimals = self.mint.decimals;
        let cpi = CpiContext::new_with_signer(
            self.token_program.key(),
            TransferChecked {
                from: self.vault.to_account_info(),
                mint: self.mint.to_account_info(),
                to: self.destination.to_account_info(),
                authority: self.schedule.to_account_info(),
            },
            signer_seeds,
        );
        transfer_checked(cpi, releasable, decimals)?;

        Ok(())
    }
}
