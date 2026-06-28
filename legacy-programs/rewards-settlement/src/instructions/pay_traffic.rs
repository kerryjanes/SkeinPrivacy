use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    burn_checked, transfer_checked, BurnChecked, Mint, TokenAccount, TokenInterface,
    TransferChecked,
};
use weft_primitives::split_payment_bps;

use crate::{
    constants::*,
    error::SettlementError,
    external::{load_protocol_split, GOVERNANCE_ID, PROTOCOL_CONFIG_SEED},
    state::Distributor,
};

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
    /// CHECK: PDA address is constrained to governance `[protocol_config]`; owner,
    /// discriminator, and split offsets are validated before reading.
    #[account(
        seeds = [PROTOCOL_CONFIG_SEED],
        bump,
        seeds::program = GOVERNANCE_ID,
    )]
    pub protocol_config: UncheckedAccount<'info>,
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
    /// Split a user traffic payment per the governed `ProtocolConfig`: nodes share
    /// → reward vault, burn share burned, remainder → treasury (default 70/20/10).
    pub fn pay_traffic(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);
        let protocol_split = load_protocol_split(&self.protocol_config.to_account_info())?;
        let split = split_payment_bps(amount, protocol_split.nodes_bps, protocol_split.burn_bps);
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
