use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::GovError,
    state::{GovernanceConfig, Proposal, ProposalState},
};

#[derive(Accounts)]
pub struct ActivateProposal<'info> {
    pub proposer: Signer<'info>,
    #[account(seeds = [GOVERNANCE_CONFIG_SEED], bump = governance_config.bump)]
    pub governance_config: Account<'info, GovernanceConfig>,
    #[account(
        mut,
        seeds = [PROPOSAL_SEED, &proposal.id.to_le_bytes()],
        bump = proposal.bump,
        constraint = proposal.proposer == proposer.key() @ GovError::Unauthorized,
    )]
    pub proposal: Account<'info, Proposal>,
}

impl ActivateProposal<'_> {
    pub fn activate_proposal(&mut self) -> Result<()> {
        require!(
            self.proposal.state == ProposalState::Draft,
            GovError::InvalidState
        );
        let now = Clock::get()?.unix_timestamp;
        self.proposal.voting_starts_at = now;
        self.proposal.voting_ends_at = now
            .checked_add(self.governance_config.voting_period_seconds)
            .ok_or(GovError::MathOverflow)?;
        self.proposal.quorum = self.governance_config.default_quorum;
        self.proposal.approval_threshold_bps =
            self.governance_config.default_approval_threshold_bps;
        self.proposal.state = ProposalState::Voting;
        Ok(())
    }
}
