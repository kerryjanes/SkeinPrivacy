use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    burn_checked, transfer_checked, BurnChecked, Mint, TokenAccount, TokenInterface,
    TransferChecked,
};
use weft_primitives::split_payment;

use crate::{constants::*, error::SettlementError, state::Distributor};

#[derive(Accounts)]
pub struct PayTraffic<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        has_one = reward_mint,
        has_one = reward_vault,
        has_one = treasury,
    )]
    pub distributor: Account<'info, Distributor>,
    #[account(mut)]
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = reward_mint, token::authority = payer)]
    pub payer_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub treasury: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl PayTraffic<'_> {
    /// Split a user traffic payment 70/20/10: 70% → reward vault, 20% burned, 10% → treasury.
    pub fn pay_traffic(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);
        let split = split_payment(amount);
        let dec = self.reward_mint.decimals;
        let program = self.token_program.key();

        if split.nodes > 0 {
            transfer_checked(
                CpiContext::new(
                    program,
                    TransferChecked {
                        from: self.payer_token_account.to_account_info(),
                        mint: self.reward_mint.to_account_info(),
                        to: self.reward_vault.to_account_info(),
                        authority: self.payer.to_account_info(),
                    },
                ),
                split.nodes,
                dec,
            )?;
        }
        if split.treasury > 0 {
            transfer_checked(
                CpiContext::new(
                    program,
                    TransferChecked {
                        from: self.payer_token_account.to_account_info(),
                        mint: self.reward_mint.to_account_info(),
                        to: self.treasury.to_account_info(),
                        authority: self.payer.to_account_info(),
                    },
                ),
                split.treasury,
                dec,
            )?;
        }
        if split.burn > 0 {
            burn_checked(
                CpiContext::new(
                    program,
                    BurnChecked {
                        mint: self.reward_mint.to_account_info(),
                        from: self.payer_token_account.to_account_info(),
                        authority: self.payer.to_account_info(),
                    },
                ),
                split.burn,
                dec,
            )?;
        }
        Ok(())
    }
}
