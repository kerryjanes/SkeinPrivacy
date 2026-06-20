use anchor_lang::prelude::*;

use crate::constants::AUTHORITY_SEED;

/// Mirror a node's reputation multiplier into `NodeState` via gated CPI into
/// node-registry. No-op when the node account is absent.
pub fn mirror_reputation<'info>(
    node_registry_program: &AccountInfo<'info>,
    program_authority: &AccountInfo<'info>,
    registry: &AccountInfo<'info>,
    node: Option<&AccountInfo<'info>>,
    authority_bump: u8,
    multiplier_bps: u16,
) -> Result<()> {
    let Some(node) = node else {
        return Ok(());
    };
    let seeds: &[&[&[u8]]] = &[&[AUTHORITY_SEED, &[authority_bump]]];
    let cpi = CpiContext::new_with_signer(
        node_registry_program.key(),
        node_registry::cpi::accounts::SetReputation {
            reputation_authority: program_authority.clone(),
            registry: registry.clone(),
            node: node.clone(),
        },
        seeds,
    );
    node_registry::cpi::set_reputation(cpi, multiplier_bps)
}
