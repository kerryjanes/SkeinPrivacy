use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::GovError,
    state::{Proposal, ProposalState, ProposalTransaction, TxAccountMeta},
};

#[derive(Accounts)]
#[instruction(index: u16)]
pub struct AddTransaction<'info> {
    #[account(mut)]
    pub proposer: Signer<'info>,
    #[account(
        mut,
        seeds = [PROPOSAL_SEED, &proposal.id.to_le_bytes()],
        bump = proposal.bump,
        constraint = proposal.proposer == proposer.key() @ GovError::Unauthorized,
    )]
    pub proposal: Account<'info, Proposal>,
    #[account(
        init,
        payer = proposer,
        space = 8 + ProposalTransaction::INIT_SPACE,
        seeds = [PROPOSAL_TX_SEED, proposal.key().as_ref(), &index.to_le_bytes()],
        bump
    )]
    pub proposal_transaction: Account<'info, ProposalTransaction>,
    pub system_program: Program<'info, System>,
}

impl AddTransaction<'_> {
    pub fn add_transaction(
        &mut self,
        index: u16,
        program_id: Pubkey,
        accounts: Vec<TxAccountMeta>,
        data: Vec<u8>,
        bump: u8,
    ) -> Result<()> {
        require!(
            self.proposal.state == ProposalState::Draft,
            GovError::InvalidState
        );
        // Sequential append keeps indices dense for execution.
        require!(
            index == self.proposal.transaction_count,
            GovError::InvalidState
        );
        require!(accounts.len() <= MAX_TX_ACCOUNTS, GovError::TooManyAccounts);
        require!(data.len() <= MAX_TX_DATA, GovError::DataTooLong);
        self.proposal_transaction.set_inner(ProposalTransaction {
            proposal: self.proposal.key(),
            index,
            program_id,
            accounts,
            data,
            executed: false,
            bump,
        });
        self.proposal.transaction_count = self
            .proposal
            .transaction_count
            .checked_add(1)
            .ok_or(GovError::MathOverflow)?;
        Ok(())
    }
}
