use anchor_lang::prelude::*;
use mpl_bubblegum::{
    instructions::MintV2CpiBuilder,
    types::{MetadataArgsV2, TokenStandard},
    utils::get_asset_id,
};

use crate::{
    constants::*,
    error::RegistryError,
    state::{NodeState, Registry, TreeShard},
};

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct Register<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,

    #[account(
        mut,
        seeds = [REGISTRY_SEED],
        bump = registry.bump,
        constraint = !registry.paused @ RegistryError::Paused,
    )]
    pub registry: Account<'info, Registry>,

    #[account(
        mut,
        seeds = [TREE_SEED, &tree_shard.index.to_le_bytes()],
        bump = tree_shard.bump,
        constraint = tree_shard.merkle_tree == registry.active_tree @ RegistryError::InvalidTree,
        constraint = !tree_shard.full @ RegistryError::TreeFull,
    )]
    pub tree_shard: Account<'info, TreeShard>,

    #[account(
        init,
        payer = operator,
        space = 8 + NodeState::INIT_SPACE,
        seeds = [NODE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump
    )]
    pub node: Account<'info, NodeState>,

    // --- Bubblegum V2 mintV2 accounts ---
    /// CHECK: Bubblegum tree-config PDA; validated by the Bubblegum program.
    #[account(mut)]
    pub tree_config: UncheckedAccount<'info>,
    /// CHECK: must be the registry's active tree; validated by the Bubblegum program.
    #[account(mut, address = registry.active_tree)]
    pub merkle_tree: UncheckedAccount<'info>,
    /// CHECK: the registry's MPL-Core collection.
    #[account(mut, address = registry.collection)]
    pub core_collection: UncheckedAccount<'info>,
    /// CHECK: Bubblegum's mpl-core CPI signer PDA.
    pub mpl_core_cpi_signer: UncheckedAccount<'info>,
    /// CHECK: mpl-noop / log wrapper program.
    pub log_wrapper: UncheckedAccount<'info>,
    /// CHECK: mpl-account-compression program.
    pub compression_program: UncheckedAccount<'info>,
    /// CHECK: mpl-core program.
    pub mpl_core_program: UncheckedAccount<'info>,
    /// CHECK: the Bubblegum program.
    #[account(address = mpl_bubblegum::ID)]
    pub bubblegum_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

impl Register<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &mut self,
        node_id: u64,
        geo: u32,
        capabilities: u32,
        endpoint_hash: [u8; 32],
        availability: u8,
        metadata_uri: String,
        bump: u8,
    ) -> Result<()> {
        // Validate before any CPI.
        require!(
            weft_primitives::geo_is_valid(geo),
            RegistryError::InvalidGeo
        );
        require!(
            weft_primitives::capabilities_valid(capabilities),
            RegistryError::InvalidCapabilities
        );
        require!(
            weft_primitives::availability_is_valid(availability),
            RegistryError::InvalidAvailability
        );

        let merkle_tree = self.registry.active_tree;
        let leaf_nonce = self.tree_shard.minted;
        let asset_id = get_asset_id(&merkle_tree, leaf_nonce);
        let now = Clock::get()?.unix_timestamp;

        self.node.set_inner(NodeState {
            operator: self.operator.key(),
            node_id,
            asset_id,
            merkle_tree,
            leaf_nonce,
            geo,
            capabilities,
            endpoint_hash,
            availability,
            status: STATUS_ACTIVE,
            registered_at: now,
            updated_at: now,
            reputation: 0,
            stake_amount: 0,
            bump,
            sequence: 0, // stamped below once the registry counter is bumped
        });

        let metadata = MetadataArgsV2 {
            name: format!("Weft Node #{node_id}"),
            symbol: "WEFTODE".to_string(),
            uri: metadata_uri,
            seller_fee_basis_points: 0,
            primary_sale_happened: false,
            is_mutable: true,
            token_standard: Some(TokenStandard::NonFungible),
            creators: vec![],
            collection: Some(self.registry.collection),
        };

        // Bind AccountInfos to locals (the CPI builder stores references).
        let bubblegum = self.bubblegum_program.to_account_info();
        let tree_config = self.tree_config.to_account_info();
        let operator = self.operator.to_account_info();
        let registry_ai = self.registry.to_account_info();
        let merkle_tree_ai = self.merkle_tree.to_account_info();
        let core_collection = self.core_collection.to_account_info();
        let cpi_signer = self.mpl_core_cpi_signer.to_account_info();
        let log_wrapper = self.log_wrapper.to_account_info();
        let compression = self.compression_program.to_account_info();
        let core_program = self.mpl_core_program.to_account_info();
        let system = self.system_program.to_account_info();

        let registry_bump = self.registry.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[REGISTRY_SEED, &[registry_bump]]];

        MintV2CpiBuilder::new(&bubblegum)
            .tree_config(&tree_config)
            .payer(&operator)
            .tree_creator_or_delegate(Some(&registry_ai))
            .collection_authority(Some(&registry_ai))
            .leaf_owner(&operator)
            .merkle_tree(&merkle_tree_ai)
            .core_collection(Some(&core_collection))
            .mpl_core_cpi_signer(Some(&cpi_signer))
            .log_wrapper(&log_wrapper)
            .compression_program(&compression)
            .mpl_core_program(&core_program)
            .system_program(&system)
            .metadata(metadata)
            .invoke_signed(signer_seeds)?;

        // Account for the new leaf.
        self.tree_shard.minted = self
            .tree_shard
            .minted
            .checked_add(1)
            .ok_or(RegistryError::MathOverflow)?;
        if self.tree_shard.minted >= self.tree_shard.capacity {
            self.tree_shard.full = true;
        }
        self.registry.node_count = self
            .registry
            .node_count
            .checked_add(1)
            .ok_or(RegistryError::MathOverflow)?;
        // Stamp the node's global registration order (monotonic; never rewound by a
        // deregister) for the M8 cold-start bonus.
        self.registry.node_sequence = self
            .registry
            .node_sequence
            .checked_add(1)
            .ok_or(RegistryError::MathOverflow)?;
        self.node.sequence = self.registry.node_sequence;
        Ok(())
    }
}
