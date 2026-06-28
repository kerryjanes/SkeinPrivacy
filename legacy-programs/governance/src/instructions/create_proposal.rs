use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::GovError,
    external::load_stake_position,
    state::{GovernanceConfig, Proposal, ProposalState},
};

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct CreateProposal<'info> {
    #[account(mut)]
    pub proposer: Signer<'info>,
    #[account(mut, seeds = [GOVERNANCE_CONFIG_SEED], bump = governance_config.bump)]
    pub governance_config: Account<'info, GovernanceConfig>,
    /// CHECK: validated manually against staking program owner, PDA seeds, discriminator,
    /// operator, and node id before reading stake amount.
    pub position: UncheckedAccount<'info>,
    #[account(
        init,
        payer = proposer,
        space = 8 + Proposal::INIT_SPACE,
        seeds = [PROPOSAL_SEED, &governance_config.proposal_count.to_le_bytes()],
        bump
    )]
    pub proposal: Account<'info, Proposal>,
    pub system_program: Program<'info, System>,
}

impl CreateProposal<'_> {
    pub fn create_proposal(&mut self, _node_id: u64, name: String, bump: u8) -> Result<()> {
        require!(name.len() <= MAX_PROPOSAL_NAME, GovError::DataTooLong);
        let position = load_stake_position(
            &self.position.to_account_info(),
            &self.proposer.key(),
            _node_id,
        )?;
        require!(
            position.amount >= self.governance_config.min_proposal_stake,
            GovError::InsufficientProposalStake
        );
        let now = Clock::get()?.unix_timestamp;
        let id = self.governance_config.proposal_count;
        self.proposal.set_inner(Proposal {
            id,
            proposer: self.proposer.key(),
            state: ProposalState::Draft,
            created_at: now,
            voting_starts_at: 0,
            voting_ends_at: 0,
            execution_eta: 0,
            quorum: 0,
            approval_threshold_bps: 0,
            yes: 0,
            no: 0,
            abstain: 0,
            transaction_count: 0,
            executed_count: 0,
            name,
            bump,
        });
        self.governance_config.proposal_count = id.checked_add(1).ok_or(GovError::MathOverflow)?;
        Ok(())
    }
}
