use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::GovError,
    external::load_stake_position,
    state::{Proposal, ProposalState, VoteKind, VoteRecord},
};

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct CastVote<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,
    #[account(
        mut,
        seeds = [PROPOSAL_SEED, &proposal.id.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Account<'info, Proposal>,
    /// CHECK: validated manually against staking program owner, PDA seeds, discriminator,
    /// operator, and node id before reading vote weight.
    pub position: UncheckedAccount<'info>,
    /// `init` makes each position votable exactly once per proposal (double-vote guard).
    #[account(
        init,
        payer = voter,
        space = 8 + VoteRecord::INIT_SPACE,
        seeds = [VOTE_SEED, proposal.key().as_ref(), position.key().as_ref()],
        bump
    )]
    pub vote_record: Account<'info, VoteRecord>,
    pub system_program: Program<'info, System>,
}

impl CastVote<'_> {
    pub fn cast_vote(&mut self, node_id: u64, vote: VoteKind, bump: u8) -> Result<()> {
        require!(
            self.proposal.state == ProposalState::Voting,
            GovError::InvalidState
        );
        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= self.proposal.voting_starts_at && now < self.proposal.voting_ends_at,
            GovError::VotingClosed
        );
        let position =
            load_stake_position(&self.position.to_account_info(), &self.voter.key(), node_id)?;
        // Stake must stay locked until voting ends, so it can't be unbonded mid-vote.
        require!(
            position.locked_until >= self.proposal.voting_ends_at,
            GovError::PositionUnlockedBeforeVoteEnds
        );

        let weight = position.amount;
        let w = weight as u128;
        match vote {
            VoteKind::Yes => {
                self.proposal.yes = self
                    .proposal
                    .yes
                    .checked_add(w)
                    .ok_or(GovError::MathOverflow)?
            }
            VoteKind::No => {
                self.proposal.no = self
                    .proposal
                    .no
                    .checked_add(w)
                    .ok_or(GovError::MathOverflow)?
            }
            VoteKind::Abstain => {
                self.proposal.abstain = self
                    .proposal
                    .abstain
                    .checked_add(w)
                    .ok_or(GovError::MathOverflow)?
            }
        }
        self.vote_record.set_inner(VoteRecord {
            proposal: self.proposal.key(),
            position: self.position.key(),
            operator: self.voter.key(),
            node_id,
            weight,
            vote,
            bump,
        });
        Ok(())
    }
}
