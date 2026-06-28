use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{constants::*, error::VestingError, state::VestingSchedule};

#[derive(Accounts)]
pub struct Revoke<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [SCHEDULE_SEED, schedule.beneficiary.as_ref(), &schedule.schedule_id.to_le_bytes()],
        bump = schedule.bump,
        has_one = authority @ VestingError::Unauthorized,
        has_one = mint,
        has_one = vault,
        constraint = schedule.revocable @ VestingError::NotRevocable,
        constraint = !schedule.revoked @ VestingError::AlreadyRevoked,
    )]
    pub schedule: Account<'info, VestingSchedule>,

    pub mint: InterfaceAccount<'info, Mint>,

    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    /// Authority-chosen destination for the reclaimed unvested remainder.
    #[account(mut, token::mint = mint)]
    pub destination: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
}

impl<'info> Revoke<'info> {
    pub fn revoke(&mut self) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let vault_amount = self.vault.amount;

        // Vested up to now (revocation freezes accrual here).
        let vested_now = self.schedule.vested(now);
        // Still owed to the beneficiary: vested but not yet claimed.
        let owed = vested_now
            .checked_sub(self.schedule.released_amount)
            .ok_or(VestingError::MathOverflow)?;
        // The vault currently holds `total - released`; reclaim the unvested rest.
        let reclaim = vault_amount
            .checked_sub(owed)
            .ok_or(VestingError::MathOverflow)?;

        self.schedule.revoked = true;
        self.schedule.revoked_ts = now;

        if reclaim > 0 {
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
            transfer_checked(cpi, reclaim, decimals)?;
        }

        Ok(())
    }
}
