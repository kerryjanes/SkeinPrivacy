use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{constants::*, error::VestingError, state::VestingSchedule};

#[derive(Accounts)]
#[instruction(schedule_id: u64)]
pub struct CreateSchedule<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,

    /// CHECK: stored as the schedule beneficiary; need not sign at creation.
    pub beneficiary: UncheckedAccount<'info>,

    /// CHECK: stored as the revoke authority; need not sign at creation.
    pub authority: UncheckedAccount<'info>,

    pub mint: InterfaceAccount<'info, Mint>,

    #[account(
        init,
        payer = funder,
        space = 8 + VestingSchedule::INIT_SPACE,
        seeds = [SCHEDULE_SEED, beneficiary.key().as_ref(), &schedule_id.to_le_bytes()],
        bump
    )]
    pub schedule: Account<'info, VestingSchedule>,

    #[account(
        init,
        payer = funder,
        token::mint = mint,
        token::authority = schedule,
        seeds = [VAULT_SEED, schedule.key().as_ref()],
        bump
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = mint,
        token::authority = funder,
    )]
    pub funder_token_account: InterfaceAccount<'info, TokenAccount>,

    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> CreateSchedule<'info> {
    #[allow(clippy::too_many_arguments)]
    pub fn create_schedule(
        &mut self,
        schedule_id: u64,
        total_amount: u64,
        cliff_unlock_amount: u64,
        start_ts: i64,
        cliff_duration: i64,
        duration: i64,
        revocable: bool,
        bump: u8,
    ) -> Result<()> {
        require!(total_amount > 0, VestingError::InvalidParams);
        require!(duration > 0, VestingError::InvalidParams);
        require!(cliff_duration >= 0, VestingError::InvalidParams);
        require!(cliff_duration <= duration, VestingError::InvalidParams);
        require!(
            cliff_unlock_amount <= total_amount,
            VestingError::InvalidParams
        );

        let vesting = VestingSchedule {
            beneficiary: self.beneficiary.key(),
            authority: self.authority.key(),
            mint: self.mint.key(),
            vault: self.vault.key(),
            schedule_id,
            total_amount,
            cliff_unlock_amount,
            released_amount: 0,
            start_ts,
            cliff_duration,
            duration,
            revocable,
            revoked: false,
            revoked_ts: 0,
            bump,
        };
        self.schedule.set_inner(vesting);

        // Fund the vault atomically with creation: the funder must hold `total_amount`.
        let cpi = CpiContext::new(
            self.token_program.key(),
            TransferChecked {
                from: self.funder_token_account.to_account_info(),
                mint: self.mint.to_account_info(),
                to: self.vault.to_account_info(),
                authority: self.funder.to_account_info(),
            },
        );
        transfer_checked(cpi, total_amount, self.mint.decimals)?;

        Ok(())
    }
}
