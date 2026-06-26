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
    state::{Distributor, PaymentEscrow},
};

fn init_or_validate_escrow(
    escrow: &mut PaymentEscrow,
    owner: Pubkey,
    mint: Pubkey,
    vault: Pubkey,
    bump: u8,
) -> Result<()> {
    if escrow.owner == Pubkey::default() {
        escrow.owner = owner;
        escrow.mint = mint;
        escrow.vault = vault;
        escrow.bump = bump;
        return Ok(());
    }

    require!(escrow.owner == owner, SettlementError::Unauthorized);
    require!(escrow.mint == mint, SettlementError::InvalidEscrow);
    require!(escrow.vault == vault, SettlementError::InvalidEscrow);
    Ok(())
}

#[derive(Accounts)]
pub struct DepositEscrow<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        has_one = reward_mint,
    )]
    pub distributor: Account<'info, Distributor>,
    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + PaymentEscrow::INIT_SPACE,
        seeds = [ESCROW_SEED, owner.key().as_ref()],
        bump
    )]
    pub escrow: Account<'info, PaymentEscrow>,
    #[account(
        init_if_needed,
        payer = owner,
        token::mint = reward_mint,
        token::authority = escrow,
        seeds = [ESCROW_VAULT_SEED, owner.key().as_ref()],
        bump
    )]
    pub escrow_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = reward_mint, token::authority = owner)]
    pub owner_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl DepositEscrow<'_> {
    pub fn deposit_escrow(&mut self, amount: u64, escrow_bump: u8) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);
        init_or_validate_escrow(
            &mut self.escrow,
            self.owner.key(),
            self.reward_mint.key(),
            self.escrow_vault.key(),
            escrow_bump,
        )?;

        let next_balance = self
            .escrow
            .balance
            .checked_add(amount)
            .ok_or(SettlementError::MathOverflow)?;
        let next_deposited = self
            .escrow
            .total_deposited
            .checked_add(amount)
            .ok_or(SettlementError::MathOverflow)?;

        transfer_checked(
            CpiContext::new(
                self.token_program.key(),
                TransferChecked {
                    from: self.owner_token_account.to_account_info(),
                    mint: self.reward_mint.to_account_info(),
                    to: self.escrow_vault.to_account_info(),
                    authority: self.owner.to_account_info(),
                },
            ),
            amount,
            self.reward_mint.decimals,
        )?;

        self.escrow.balance = next_balance;
        self.escrow.total_deposited = next_deposited;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct PayTrafficFromEscrow<'info> {
    pub owner: Signer<'info>,
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
    #[account(
        mut,
        seeds = [ESCROW_SEED, owner.key().as_ref()],
        bump = escrow.bump,
        constraint = escrow.owner == owner.key() @ SettlementError::Unauthorized,
        constraint = escrow.mint == reward_mint.key() @ SettlementError::InvalidEscrow,
        constraint = escrow.vault == escrow_vault.key() @ SettlementError::InvalidEscrow,
    )]
    pub escrow: Account<'info, PaymentEscrow>,
    #[account(mut, token::mint = reward_mint, token::authority = escrow)]
    pub escrow_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub treasury: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl PayTrafficFromEscrow<'_> {
    /// Settle prepaid traffic from the user's escrow with the same governed split as `pay_traffic`.
    pub fn pay_traffic_from_escrow(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);
        require!(
            self.escrow.balance >= amount,
            SettlementError::InsufficientEscrow
        );

        let protocol_split = load_protocol_split(&self.protocol_config.to_account_info())?;
        let split = split_payment_bps(amount, protocol_split.nodes_bps, protocol_split.burn_bps);
        let dec = self.reward_mint.decimals;
        let program = self.token_program.key();
        let owner_key = self.owner.key();
        let signer_seeds: &[&[&[u8]]] = &[&[ESCROW_SEED, owner_key.as_ref(), &[self.escrow.bump]]];

        if split.nodes > 0 {
            transfer_checked(
                CpiContext::new(
                    program,
                    TransferChecked {
                        from: self.escrow_vault.to_account_info(),
                        mint: self.reward_mint.to_account_info(),
                        to: self.reward_vault.to_account_info(),
                        authority: self.escrow.to_account_info(),
                    },
                )
                .with_signer(signer_seeds),
                split.nodes,
                dec,
            )?;
        }
        if split.treasury > 0 {
            transfer_checked(
                CpiContext::new(
                    program,
                    TransferChecked {
                        from: self.escrow_vault.to_account_info(),
                        mint: self.reward_mint.to_account_info(),
                        to: self.treasury.to_account_info(),
                        authority: self.escrow.to_account_info(),
                    },
                )
                .with_signer(signer_seeds),
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
                        from: self.escrow_vault.to_account_info(),
                        authority: self.escrow.to_account_info(),
                    },
                )
                .with_signer(signer_seeds),
                split.burn,
                dec,
            )?;
        }

        self.escrow.balance = self
            .escrow
            .balance
            .checked_sub(amount)
            .ok_or(SettlementError::MathOverflow)?;
        self.escrow.total_spent = self
            .escrow
            .total_spent
            .checked_add(amount)
            .ok_or(SettlementError::MathOverflow)?;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct WithdrawEscrow<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, owner.key().as_ref()],
        bump = escrow.bump,
        constraint = escrow.owner == owner.key() @ SettlementError::Unauthorized,
        constraint = escrow.mint == reward_mint.key() @ SettlementError::InvalidEscrow,
        constraint = escrow.vault == escrow_vault.key() @ SettlementError::InvalidEscrow,
    )]
    pub escrow: Account<'info, PaymentEscrow>,
    #[account(mut, token::mint = reward_mint, token::authority = escrow)]
    pub escrow_vault: InterfaceAccount<'info, TokenAccount>,
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = reward_mint, token::authority = owner)]
    pub owner_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl WithdrawEscrow<'_> {
    pub fn withdraw_escrow(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, SettlementError::ZeroAmount);
        require!(
            self.escrow.balance >= amount,
            SettlementError::InsufficientEscrow
        );

        let owner_key = self.owner.key();
        let signer_seeds: &[&[&[u8]]] = &[&[ESCROW_SEED, owner_key.as_ref(), &[self.escrow.bump]]];

        transfer_checked(
            CpiContext::new(
                self.token_program.key(),
                TransferChecked {
                    from: self.escrow_vault.to_account_info(),
                    mint: self.reward_mint.to_account_info(),
                    to: self.owner_token_account.to_account_info(),
                    authority: self.escrow.to_account_info(),
                },
            )
            .with_signer(signer_seeds),
            amount,
            self.reward_mint.decimals,
        )?;

        self.escrow.balance = self
            .escrow
            .balance
            .checked_sub(amount)
            .ok_or(SettlementError::MathOverflow)?;
        Ok(())
    }
}
