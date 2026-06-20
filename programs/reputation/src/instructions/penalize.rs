use anchor_lang::prelude::*;
use weft_primitives::{reputation_decayed_quality_bps, reputation_multiplier_bps, BPS};

use crate::{
    constants::*,
    error::ReputationError,
    mirror::mirror_reputation,
    state::{ReputationConfig, ReputationState},
};

#[derive(Accounts)]
pub struct Penalize<'info> {
    pub oracle: Signer<'info>,
    #[account(
        seeds = [CONFIG_SEED],
        bump = config.bump,
        constraint = config.oracle == oracle.key() @ ReputationError::Unauthorized,
    )]
    pub config: Account<'info, ReputationConfig>,
    #[account(
        mut,
        seeds = [STATE_SEED, state.operator.as_ref(), &state.node_id.to_le_bytes()],
        bump = state.bump,
    )]
    pub state: Account<'info, ReputationState>,
    /// CHECK: program signer PDA for the mirror CPI.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub program_authority: UncheckedAccount<'info>,
    /// CHECK: node-registry program.
    #[account(address = node_registry::ID)]
    pub node_registry_program: UncheckedAccount<'info>,
    /// CHECK: node-registry Registry PDA.
    pub registry: UncheckedAccount<'info>,
    /// CHECK: NodeState PDA, optional.
    #[account(mut)]
    pub node: Option<UncheckedAccount<'info>>,
}

impl Penalize<'_> {
    /// Multiplicatively reduce quality toward zero (0.5x) for recorded complaints.
    pub fn penalize(&mut self, severity_bps: u32, authority_bump: u8) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let sev = severity_bps.min(BPS);
        let decayed =
            reputation_decayed_quality_bps(self.state.quality_bps, now, self.state.updated_at);
        self.state.quality_bps = (decayed as u128 * (BPS - sev) as u128 / BPS as u128) as u32;
        self.state.multiplier_bps = reputation_multiplier_bps(self.state.quality_bps);
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
