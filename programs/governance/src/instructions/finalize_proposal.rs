use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::GovError,
    state::{GovernanceConfig, Proposal, ProposalState},
};

#[derive(Accounts)]
pub struct FinalizeProposal<'info> {
    pub finalizer: Signer<'info>,
    #[account(seeds = [GOVERNANCE_CONFIG_SEED], bump = governance_config.bump)]
    pub governance_config: Account<'info, GovernanceConfig>,
    #[account(mut, seeds = [PROPOSAL_SEED, &proposal.id.to_le_bytes()], bump = proposal.bump)]
    pub proposal: Account<'info, Proposal>,
}

impl FinalizeProposal<'_> {
    pub fn finalize_proposal(&mut self) -> Result<()> {
        require!(
            self.proposal.state == ProposalState::Voting,
            GovError::InvalidState
        );
        let now = Clock::get()?.unix_timestamp;
        require!(now >= self.proposal.voting_ends_at, GovError::VotingOngoing);

        let p = &self.proposal;
        let total = p
            .yes
            .checked_add(p.no)
            .and_then(|x| x.checked_add(p.abstain))
            .ok_or(GovError::MathOverflow)?;
        let quorum_met = total >= p.quorum as u128;
        let decided = p.yes.checked_add(p.no).ok_or(GovError::MathOverflow)?;
        let threshold_met = decided > 0
            && p.yes * weft_primitives::BPS as u128 / decided >= p.approval_threshold_bps as u128;

        if quorum_met && threshold_met {
            self.proposal.execution_eta = now
                .checked_add(self.governance_config.execution_delay_seconds)
                .ok_or(GovError::MathOverflow)?;
            self.proposal.state = ProposalState::Succeeded;
        } else {
            self.proposal.state = ProposalState::Defeated;
        }
        Ok(())
    }
}
