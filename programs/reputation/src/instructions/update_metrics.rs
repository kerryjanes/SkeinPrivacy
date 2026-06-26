use anchor_lang::prelude::*;
use weft_primitives::{
    reputation_decayed_quality_bps, reputation_ema, reputation_multiplier_bps,
    reputation_quality_bps, BPS, REPUTATION_BASELINE_QUALITY_BPS, REPUTATION_EMA_ALPHA_BPS,
};

use crate::{
    constants::*,
    error::ReputationError,
    external::NODE_REGISTRY_ID,
    mirror::mirror_reputation,
    state::{ReputationConfig, ReputationState},
};

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct UpdateMetrics<'info> {
    #[account(mut)]
    pub oracle: Signer<'info>,
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = config.oracle == oracle.key() @ ReputationError::Unauthorized,
    )]
    pub config: Account<'info, ReputationConfig>,
    /// CHECK: the node owner; binds the reputation state PDA.
    pub operator: UncheckedAccount<'info>,
    #[account(
        init_if_needed,
        payer = oracle,
        space = 8 + ReputationState::INIT_SPACE,
        seeds = [STATE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump
    )]
    pub state: Account<'info, ReputationState>,
    /// CHECK: program signer PDA for the mirror CPI.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub program_authority: UncheckedAccount<'info>,
    /// CHECK: node-registry program.
    #[account(address = NODE_REGISTRY_ID)]
    pub node_registry_program: UncheckedAccount<'info>,
    /// CHECK: node-registry Registry PDA.
    pub registry: UncheckedAccount<'info>,
    /// CHECK: NodeState PDA, optional.
    #[account(mut)]
    pub node: Option<UncheckedAccount<'info>>,
    pub system_program: Program<'info, System>,
}

impl UpdateMetrics<'_> {
    pub fn update_metrics(
        &mut self,
        node_id: u64,
        uptime_bps: u32,
        speed_bps: u32,
        review_bps: u32,
        state_bump: u8,
        authority_bump: u8,
    ) -> Result<()> {
        require!(
            uptime_bps <= BPS && speed_bps <= BPS && review_bps <= BPS,
            ReputationError::InvalidMetric
        );
        let now = Clock::get()?.unix_timestamp;

        self.state.operator = self.operator.key();
        self.state.node_id = node_id;
        self.state.bump = state_bump;

        // First update starts neutral (1.0x); later updates decay stale state first.
        let prior = if self.state.updated_at == 0 {
            REPUTATION_BASELINE_QUALITY_BPS
        } else {
            reputation_decayed_quality_bps(self.state.quality_bps, now, self.state.updated_at)
        };
        let sample = reputation_quality_bps(uptime_bps, speed_bps, review_bps);
        self.state.quality_bps = reputation_ema(prior, sample, REPUTATION_EMA_ALPHA_BPS);
        self.state.multiplier_bps = reputation_multiplier_bps(self.state.quality_bps);
        self.state.uptime_bps = uptime_bps;
        self.state.speed_bps = speed_bps;
        self.state.review_bps = review_bps;
        self.state.updated_at = now;

        let nrp = self.node_registry_program.to_account_info();
        let pa = self.program_authority.to_account_info();
        let reg = self.registry.to_account_info();
        let node_ai = self.node.as_ref().map(|n| n.to_account_info());
        mirror_reputation(
            &nrp,
            &pa,
            &reg,
            node_ai.as_ref(),
            authority_bump,
            self.state.multiplier_bps as u16,
        )
    }
}
