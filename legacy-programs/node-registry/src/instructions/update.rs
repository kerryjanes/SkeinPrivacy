use anchor_lang::prelude::*;

use crate::{constants::*, error::RegistryError, state::NodeState};

#[derive(Accounts)]
pub struct UpdateNode<'info> {
    pub operator: Signer<'info>,

    #[account(
        mut,
        seeds = [NODE_SEED, operator.key().as_ref(), &node.node_id.to_le_bytes()],
        bump = node.bump,
        has_one = operator @ RegistryError::Unauthorized,
    )]
    pub node: Account<'info, NodeState>,
}

impl UpdateNode<'_> {
    pub fn update(
        &mut self,
        geo: Option<u32>,
        capabilities: Option<u32>,
        endpoint_hash: Option<[u8; 32]>,
        availability: Option<u8>,
    ) -> Result<()> {
        require!(
            self.node.status == STATUS_ACTIVE,
            RegistryError::NodeNotActive
        );

        if let Some(geo) = geo {
            require!(
                weft_primitives::geo_is_valid(geo),
                RegistryError::InvalidGeo
            );
            self.node.geo = geo;
        }
        if let Some(capabilities) = capabilities {
            require!(
                weft_primitives::capabilities_valid(capabilities),
                RegistryError::InvalidCapabilities
            );
            self.node.capabilities = capabilities;
        }
        if let Some(endpoint_hash) = endpoint_hash {
            self.node.endpoint_hash = endpoint_hash;
        }
        if let Some(availability) = availability {
            require!(
                weft_primitives::availability_is_valid(availability),
                RegistryError::InvalidAvailability
            );
            self.node.availability = availability;
        }
        self.node.updated_at = Clock::get()?.unix_timestamp;
        Ok(())
    }
}
