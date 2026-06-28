use anchor_lang::prelude::*;

use crate::constants::AUTHORITY_SEED;
use crate::external::invoke_set_stake;

/// Mirror a node's staked balance into `NodeState` via gated CPI into
/// node-registry. No-op when the node account is absent (stake may precede
/// registration or follow deregistration).
#[allow(clippy::too_many_arguments)]
pub fn mirror_stake<'info>(
    node_registry_program: &AccountInfo<'info>,
    program_authority: &AccountInfo<'info>,
    registry: &AccountInfo<'info>,
    node: Option<&AccountInfo<'info>>,
    authority_bump: u8,
    amount: u64,
) -> Result<()> {
    let Some(node) = node else {
        return Ok(());
    };
    let seeds: &[&[&[u8]]] = &[&[AUTHORITY_SEED, &[authority_bump]]];
    invoke_set_stake(
        node_registry_program,
        program_authority,
        registry,
        node,
        seeds,
        amount,
    )
}
