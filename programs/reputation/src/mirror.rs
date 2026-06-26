use anchor_lang::prelude::*;

use crate::constants::AUTHORITY_SEED;
use crate::external::invoke_set_reputation;

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
    invoke_set_reputation(
        node_registry_program,
        program_authority,
        registry,
        node,
        seeds,
        multiplier_bps,
    )
}
