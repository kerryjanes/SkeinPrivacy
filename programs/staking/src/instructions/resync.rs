use anchor_lang::prelude::*;

use crate::{constants::*, mirror::mirror_stake, state::StakePosition};

/// Permissionless re-push of the position's amount into `NodeState` (heals any
/// mirror divergence, e.g. after a node was registered post-stake).
#[derive(Accounts)]
pub struct Resync<'info> {
    #[account(
        seeds = [STAKE_SEED, position.operator.as_ref(), &position.node_id.to_le_bytes()],
        bump = position.bump,
    )]
    pub position: Account<'info, StakePosition>,
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
        mirror_stake(
            &nrp,
            &pa,
            &reg,
            node_ai.as_ref(),
            authority_bump,
            self.position.amount,
        )
    }
}
