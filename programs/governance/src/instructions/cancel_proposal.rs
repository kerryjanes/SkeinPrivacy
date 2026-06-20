use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::GovError,
    state::{Proposal, ProposalState},
};

#[derive(Accounts)]
pub struct CancelProposal<'info> {
    pub proposer: Signer<'info>,
    #[account(
        mut,
        seeds = [PROPOSAL_SEED, &proposal.id.to_le_bytes()],
        bump = proposal.bump,
        constraint = proposal.proposer == proposer.key() @ GovError::Unauthorized,
    )]
    pub proposal: Account<'info, Proposal>,
}

impl CancelProposal<'_> {
    pub fn cancel_proposal(&mut self) -> Result<()> {
        require!(
            matches!(
                self.proposal.state,
                ProposalState::Draft | ProposalState::Voting | ProposalState::Succeeded
            ),
            GovError::InvalidState
        );
        self.proposal.state = ProposalState::Cancelled;
        Ok(())
    }
}
