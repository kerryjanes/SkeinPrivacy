use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
};

use crate::{
    constants::*,
    error::GovError,
    state::{Proposal, ProposalState, ProposalTransaction},
};

#[derive(Accounts)]
pub struct ExecuteTransaction<'info> {
    pub executor: Signer<'info>,
    #[account(mut, seeds = [PROPOSAL_SEED, &proposal.id.to_le_bytes()], bump = proposal.bump)]
    pub proposal: Account<'info, Proposal>,
    #[account(
        mut,
        seeds = [PROPOSAL_TX_SEED, proposal.key().as_ref(), &proposal_transaction.index.to_le_bytes()],
        bump = proposal_transaction.bump,
        constraint = proposal_transaction.proposal == proposal.key() @ GovError::InvalidState,
    )]
    pub proposal_transaction: Account<'info, ProposalTransaction>,
    /// CHECK: the DAO program-signer PDA; the only program-derived signer for the
    /// inner instruction. Verified by seeds.
    #[account(seeds = [GOVERNANCE_AUTHORITY_SEED], bump)]
    pub governance_authority: UncheckedAccount<'info>,
    // The instruction's target program + all referenced accounts are passed as
    // `remaining_accounts` (including `governance_authority` itself).
}

impl<'info> ExecuteTransaction<'info> {
    pub fn execute_transaction(ctx: Context<'info, Self>, authority_bump: u8) -> Result<()> {
        let accounts = &ctx.accounts;
        require!(
            accounts.proposal.state == ProposalState::Succeeded,
            GovError::InvalidState
        );
        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= accounts.proposal.execution_eta,
            GovError::TimelockNotElapsed
        );
        require!(
            !accounts.proposal_transaction.executed,
            GovError::AlreadyExecuted
        );

        let txn = &accounts.proposal_transaction;
        let metas: Vec<AccountMeta> = txn
            .accounts
            .iter()
            .map(|a| AccountMeta {
                pubkey: a.pubkey,
                is_signer: a.is_signer,
                is_writable: a.is_writable,
            })
            .collect();
        let ix = Instruction {
            program_id: txn.program_id,
            accounts: metas,
            data: txn.data.clone(),
        };

        let signer_seeds: &[&[&[u8]]] = &[&[GOVERNANCE_AUTHORITY_SEED, &[authority_bump]]];
        invoke_signed(&ix, ctx.remaining_accounts, signer_seeds)?;

        let proposal = &mut ctx.accounts.proposal;
        let txn = &mut ctx.accounts.proposal_transaction;
        txn.executed = true;
        proposal.executed_count = proposal
            .executed_count
            .checked_add(1)
            .ok_or(GovError::MathOverflow)?;
        if proposal.executed_count == proposal.transaction_count {
            proposal.state = ProposalState::Executed;
        }
        Ok(())
    }
}
