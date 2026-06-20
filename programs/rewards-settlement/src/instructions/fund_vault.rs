use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked,
};

use crate::{constants::*, error::SettlementError, state::Distributor};

#[derive(Accounts)]
pub struct FundVault<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    #[account(seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, has_one = reward_mint, has_one = reward_vault)]
    pub distributor: Account<'info, Distributor>,
    #[account(mut)]
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = reward_mint, token::authority = funder)]
    pub funder_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl FundVault<'_> {
    /// Emission top-up of the node-reward pool (e.g. from the 400M custody vault).
    pub fn fund_vault(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);
        let dec = self.reward_mint.decimals;
        transfer_checked(
            CpiContext::new(
                self.token_program.key(),
                TransferChecked {
                    from: self.funder_token_account.to_account_info(),
                    mint: self.reward_mint.to_account_info(),
                    to: self.reward_vault.to_account_info(),
                    authority: self.funder.to_account_info(),
                },
            ),
            amount,
            dec,
        )
    }
}
