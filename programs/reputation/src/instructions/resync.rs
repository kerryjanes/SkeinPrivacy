use anchor_lang::prelude::*;

use crate::{constants::*, mirror::mirror_reputation, state::ReputationState};

/// Permissionless re-push of the multiplier into `NodeState` (heals divergence).
#[derive(Accounts)]
pub struct Resync<'info> {
    #[account(
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

impl Resync<'_> {
    pub fn resync(&self, authority_bump: u8) -> Result<()> {
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
