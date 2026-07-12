#![allow(clippy::diverging_sub_expression)]

use anchor_lang::prelude::*;
use anchor_spl::token_interface::{
    burn_checked, transfer_checked, BurnChecked, Mint, TokenAccount, TokenInterface,
    TransferChecked,
};
use mpl_bubblegum::{
    instructions::MintV2CpiBuilder,
    types::{MetadataArgsV2, TokenStandard},
    utils::get_asset_id,
};
use weft_primitives::{
    availability_is_valid, capabilities_valid, geo_is_valid,
    merkle::{hash_reward_leaf, merkle_verify},
    split_payment, BPS, REPUTATION_MIN_BPS,
};

declare_id!("HBLZDwAjPKnmZ6KW1ah4qC7yMTysbpSrC5fUW98cJ1md");

pub const REGISTRY_SEED: &[u8] = b"registry";
pub const NODE_SEED: &[u8] = b"node";
pub const TREE_SEED: &[u8] = b"tree";
pub const STAKING_CONFIG_SEED: &[u8] = b"staking_config";
pub const STAKE_SEED: &[u8] = b"stake";
pub const STAKE_VAULT_SEED: &[u8] = b"stake_vault";
pub const DISTRIBUTOR_SEED: &[u8] = b"distributor";
pub const REWARD_VAULT_SEED: &[u8] = b"reward_vault";
pub const EPOCH_SEED: &[u8] = b"epoch";
pub const CLAIM_SEED: &[u8] = b"claim";
pub const ESCROW_SEED: &[u8] = b"escrow";
pub const ESCROW_VAULT_SEED: &[u8] = b"escrow_vault";

pub const STATUS_ACTIVE: u8 = 0;
pub const STATUS_SUSPENDED: u8 = 1;
pub const STATUS_DEREGISTERED: u8 = 2;
pub const MIN_LOCK_SECONDS: i64 = 0;
pub const MAX_LOCK_SECONDS: i64 = 4 * 365 * 24 * 3_600;
/// Maximum allowed merkle-tree depth (2^30 leaves) for a node cNFT tree shard.
pub const MAX_TREE_DEPTH: u32 = 30;
/// Maximum stake fraction a single dispute may seize (25%). Bounds slashing so no one
/// signature — nor a buggy or compromised slash authority — can wipe a node's whole stake.
/// Escalation needs repeated, individually-recorded disputes; the reputational penalty is
/// the soft-first signal that always applies.
pub const MAX_SLASH_BPS: u32 = 2_500;

#[program]
pub mod weft {
    use super::*;

    pub fn initialize_core(
        ctx: Context<InitializeCore>,
        unbonding_seconds: i64,
        dispute_window_seconds: i64,
        clawback_window_seconds: i64,
    ) -> Result<()> {
        ctx.accounts.initialize_core(
            unbonding_seconds,
            dispute_window_seconds,
            clawback_window_seconds,
            ctx.bumps.registry,
            ctx.bumps.staking_config,
            ctx.bumps.distributor,
        )
    }

    /// Provision a merkle-tree shard (created off-chain) and make it the active
    /// tree new node cNFTs mint into. Admin only.
    pub fn register_tree(ctx: Context<RegisterTree>, index: u16, max_depth: u32) -> Result<()> {
        ctx.accounts
            .register_tree(index, max_depth, ctx.bumps.tree_shard)
    }

    /// Point the registry at the MPL-Core collection (created off-chain with the
    /// registry PDA as update authority) that all node cNFTs belong to. Admin only.
    pub fn set_registry_collection(
        ctx: Context<ConfigureRegistry>,
        collection: Pubkey,
    ) -> Result<()> {
        ctx.accounts.registry.collection = collection;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn register_node(
        ctx: Context<RegisterNode>,
        node_id: u64,
        geo: u32,
        capabilities: u32,
        endpoint_hash: [u8; 32],
        availability: u8,
        metadata_uri: String,
    ) -> Result<()> {
        ctx.accounts.register_node(
            node_id,
            geo,
            capabilities,
            endpoint_hash,
            availability,
            metadata_uri,
            ctx.bumps.node,
        )
    }

    pub fn update_node(
        ctx: Context<UpdateNode>,
        geo: Option<u32>,
        capabilities: Option<u32>,
        endpoint_hash: Option<[u8; 32]>,
        availability: Option<u8>,
    ) -> Result<()> {
        ctx.accounts
            .update_node(geo, capabilities, endpoint_hash, availability)
    }

    pub fn deregister_node(ctx: Context<DeregisterNode>) -> Result<()> {
        ctx.accounts.deregister_node()
    }

    pub fn set_core_authority(ctx: Context<CoreAdmin>, new_authority: Pubkey) -> Result<()> {
        ctx.accounts.set_core_authority(new_authority)
    }

    pub fn set_paused(ctx: Context<CoreAdmin>, paused: bool) -> Result<()> {
        ctx.accounts.set_paused(paused)
    }

    pub fn set_poster_authority(ctx: Context<CoreAdmin>, poster_authority: Pubkey) -> Result<()> {
        ctx.accounts.distributor.poster_authority = poster_authority;
        Ok(())
    }

    pub fn set_dispute_authority(ctx: Context<CoreAdmin>, dispute_authority: Pubkey) -> Result<()> {
        ctx.accounts.distributor.dispute_authority = dispute_authority;
        ctx.accounts.staking_config.slash_authority = dispute_authority;
        Ok(())
    }

    pub fn stake(ctx: Context<Stake>, node_id: u64, amount: u64, lock_duration: i64) -> Result<()> {
        ctx.accounts
            .stake(node_id, amount, lock_duration, ctx.bumps.position)
    }

    pub fn request_unstake(ctx: Context<RequestUnstake>, _node_id: u64, amount: u64) -> Result<()> {
        ctx.accounts.request_unstake(amount)
    }

    pub fn withdraw_unstaked(ctx: Context<WithdrawUnstaked>, node_id: u64) -> Result<()> {
        ctx.accounts.withdraw_unstaked(node_id)
    }

    pub fn close_empty_stake_position(
        ctx: Context<CloseEmptyStakePosition>,
        node_id: u64,
    ) -> Result<()> {
        ctx.accounts.close_empty_stake_position(node_id)
    }

    pub fn deposit_escrow(ctx: Context<DepositEscrow>, amount: u64) -> Result<()> {
        ctx.accounts.deposit_escrow(amount, ctx.bumps.escrow)
    }

    pub fn withdraw_escrow(ctx: Context<WithdrawEscrow>, amount: u64) -> Result<()> {
        ctx.accounts.withdraw_escrow(amount)
    }

    pub fn close_escrow(ctx: Context<CloseEscrow>) -> Result<()> {
        ctx.accounts.close_escrow()
    }

    pub fn pay_traffic(ctx: Context<PayTraffic>, amount: u64) -> Result<()> {
        ctx.accounts.pay_traffic(amount)
    }

    pub fn pay_traffic_from_escrow(ctx: Context<PayTrafficFromEscrow>, amount: u64) -> Result<()> {
        ctx.accounts.pay_traffic_from_escrow(amount)
    }

    /// Delegated metered settlement: the `poster_authority` (relay/aggregator) bills a user's accrued
    /// VPN usage straight from their prepaid escrow — no per-charge owner signature. Bounded to the
    /// escrow balance and split identically to `pay_traffic_from_escrow` (70% vault / 20% burn / 10%
    /// treasury). Funds can only move into the protocol splits; the owner's unused balance stays
    /// withdrawable by the owner alone (`withdraw_escrow`), so escrow sovereignty for unspent funds is
    /// preserved.
    pub fn settle_from_escrow(ctx: Context<SettleFromEscrow>, amount: u64) -> Result<()> {
        ctx.accounts.settle_from_escrow(amount)
    }

    pub fn fund_reward_vault(ctx: Context<FundRewardVault>, amount: u64) -> Result<()> {
        ctx.accounts.fund_reward_vault(amount)
    }

    pub fn post_epoch(
        ctx: Context<PostEpoch>,
        epoch: u64,
        merkle_root: [u8; 32],
        total_reward: u64,
        num_nodes: u32,
    ) -> Result<()> {
        ctx.accounts.post_epoch(
            epoch,
            merkle_root,
            total_reward,
            num_nodes,
            ctx.bumps.epoch_distribution,
        )
    }

    pub fn claim(
        ctx: Context<Claim>,
        epoch: u64,
        node_id: u64,
        amount: u64,
        proof: Vec<[u8; 32]>,
    ) -> Result<()> {
        ctx.accounts
            .claim(epoch, node_id, amount, proof, ctx.bumps.claim_status)
    }

    pub fn dispute(
        ctx: Context<Dispute>,
        epoch: u64,
        node_id: u64,
        amount: u64,
        severity_bps: u32,
        slash_amount: u64,
    ) -> Result<()> {
        ctx.accounts.dispute(
            epoch,
            node_id,
            amount,
            severity_bps,
            slash_amount,
            ctx.bumps.claim_status,
        )
    }

    pub fn shutdown_core(ctx: Context<ShutdownCore>) -> Result<()> {
        ctx.accounts.shutdown_core()
    }
}

#[account]
#[derive(InitSpace)]
pub struct Registry {
    pub authority: Pubkey,
    pub node_count: u64,
    pub paused: bool,
    pub bump: u8,
    pub node_sequence: u64,
    /// MPL-Core collection all node cNFTs belong to (set via `set_registry_collection`).
    pub collection: Pubkey,
    /// Merkle tree new node cNFTs currently mint into (set via `register_tree`).
    pub active_tree: Pubkey,
    /// Number of tree shards provisioned so far.
    pub tree_count: u16,
}

/// One merkle-tree shard node cNFTs mint into. PDA `[TREE_SEED, index_le]`.
/// The tree itself is created off-chain (Bubblegum `createTreeV2`); this account
/// tracks how many leaves have been minted so the next `leaf_nonce` is known.
#[account]
#[derive(InitSpace)]
pub struct TreeShard {
    pub merkle_tree: Pubkey,
    pub index: u16,
    /// Leaves minted into this tree so far (the next leaf nonce).
    pub minted: u64,
    /// Maximum leaves (`2^max_depth`).
    pub capacity: u64,
    pub full: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct NodeState {
    pub operator: Pubkey,
    pub node_id: u64,
    pub geo: u32,
    pub capabilities: u32,
    pub endpoint_hash: [u8; 32],
    pub availability: u8,
    pub status: u8,
    pub registered_at: i64,
    pub updated_at: i64,
    pub reputation: u16,
    pub stake_amount: u64,
    pub bump: u8,
    pub sequence: u64,
    /// Bubblegum V2 cNFT asset id bound to this node (the ownership/identity token).
    pub asset_id: Pubkey,
    /// Tree shard holding the cNFT leaf.
    pub merkle_tree: Pubkey,
    /// Leaf index / nonce at mint time (to recompute the leaf / fetch proofs).
    pub leaf_nonce: u64,
}

#[account]
#[derive(InitSpace)]
pub struct StakingConfig {
    pub authority: Pubkey,
    pub slash_authority: Pubkey,
    pub treasury: Pubkey,
    pub mint: Pubkey,
    pub unbonding_seconds: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct StakePosition {
    pub operator: Pubkey,
    pub node_id: u64,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub amount: u64,
    pub unbonding_amount: u64,
    pub locked_until: i64,
    pub unbonding_until: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Distributor {
    pub authority: Pubkey,
    pub poster_authority: Pubkey,
    pub dispute_authority: Pubkey,
    pub reward_mint: Pubkey,
    pub reward_vault: Pubkey,
    pub treasury: Pubkey,
    pub dispute_window_seconds: i64,
    pub clawback_window_seconds: i64,
    pub current_epoch: u64,
    pub cumulative_obligated: u64,
    pub cumulative_claimed: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct EpochDistribution {
    pub epoch: u64,
    pub merkle_root: [u8; 32],
    pub total_reward: u64,
    pub total_claimed: u64,
    pub num_nodes: u32,
    pub posted_at: i64,
    pub swept: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct ClaimStatus {
    pub epoch: u64,
    pub operator: Pubkey,
    pub node_id: u64,
    pub amount: u64,
    pub claimed_at: i64,
    pub disputed: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct PaymentEscrow {
    pub owner: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub balance: u64,
    pub total_deposited: u64,
    pub total_spent: u64,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct InitializeCore<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    pub reward_mint: InterfaceAccount<'info, Mint>,
    /// CHECK: off-chain aggregator allowed to post bandwidth settlement roots.
    pub poster_authority: UncheckedAccount<'info>,
    /// CHECK: authority allowed to dispute fraudulent bandwidth leaves.
    pub dispute_authority: UncheckedAccount<'info>,
    #[account(mut, token::mint = reward_mint)]
    pub treasury: InterfaceAccount<'info, TokenAccount>,
    #[account(init, payer = authority, space = 8 + Registry::INIT_SPACE, seeds = [REGISTRY_SEED], bump)]
    pub registry: Account<'info, Registry>,
    #[account(init, payer = authority, space = 8 + StakingConfig::INIT_SPACE, seeds = [STAKING_CONFIG_SEED], bump)]
    pub staking_config: Account<'info, StakingConfig>,
    #[account(init, payer = authority, space = 8 + Distributor::INIT_SPACE, seeds = [DISTRIBUTOR_SEED], bump)]
    pub distributor: Account<'info, Distributor>,
    #[account(
        init,
        payer = authority,
        token::mint = reward_mint,
        token::authority = distributor,
        seeds = [REWARD_VAULT_SEED],
        bump
    )]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl InitializeCore<'_> {
    fn initialize_core(
        &mut self,
        unbonding_seconds: i64,
        dispute_window_seconds: i64,
        clawback_window_seconds: i64,
        registry_bump: u8,
        staking_bump: u8,
        distributor_bump: u8,
    ) -> Result<()> {
        require!(unbonding_seconds >= 0, WeftError::InvalidUnbonding);
        require!(
            dispute_window_seconds >= 0 && clawback_window_seconds >= 0,
            WeftError::InvalidWindow
        );
        let authority = self.authority.key();
        self.registry.set_inner(Registry {
            authority,
            node_count: 0,
            paused: false,
            bump: registry_bump,
            node_sequence: 0,
            // The cNFT collection + first tree are provisioned off-chain right after
            // init, then stamped via `set_registry_collection` / `register_tree`.
            collection: Pubkey::default(),
            active_tree: Pubkey::default(),
            tree_count: 0,
        });
        self.staking_config.set_inner(StakingConfig {
            authority,
            slash_authority: self.dispute_authority.key(),
            treasury: self.treasury.key(),
            mint: self.reward_mint.key(),
            unbonding_seconds,
            bump: staking_bump,
        });
        self.distributor.set_inner(Distributor {
            authority,
            poster_authority: self.poster_authority.key(),
            dispute_authority: self.dispute_authority.key(),
            reward_mint: self.reward_mint.key(),
            reward_vault: self.reward_vault.key(),
            treasury: self.treasury.key(),
            dispute_window_seconds,
            clawback_window_seconds,
            current_epoch: 0,
            cumulative_obligated: 0,
            cumulative_claimed: 0,
            bump: distributor_bump,
        });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct ConfigureRegistry<'info> {
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [REGISTRY_SEED],
        bump = registry.bump,
        constraint = registry.authority == authority.key() @ WeftError::Unauthorized,
    )]
    pub registry: Account<'info, Registry>,
}

#[derive(Accounts)]
#[instruction(index: u16)]
pub struct RegisterTree<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [REGISTRY_SEED],
        bump = registry.bump,
        constraint = registry.authority == authority.key() @ WeftError::Unauthorized,
    )]
    pub registry: Account<'info, Registry>,
    /// CHECK: the merkle tree account (created off-chain, delegated to the registry PDA).
    pub merkle_tree: UncheckedAccount<'info>,
    // `init_if_needed`: a shard PDA may already exist from a prior core (shutdown_core
    // does not close tree shards), so re-provisioning a fresh tree reuses the slot.
    // Authority-gated, so no re-init attack surface.
    #[account(
        init_if_needed,
        payer = authority,
        space = 8 + TreeShard::INIT_SPACE,
        seeds = [TREE_SEED, &index.to_le_bytes()],
        bump
    )]
    pub tree_shard: Account<'info, TreeShard>,
    pub system_program: Program<'info, System>,
}

impl RegisterTree<'_> {
    fn register_tree(&mut self, index: u16, max_depth: u32, bump: u8) -> Result<()> {
        require!(
            index == self.registry.tree_count,
            WeftError::TreeIndexMismatch
        );
        require!(
            max_depth > 0 && max_depth <= MAX_TREE_DEPTH,
            WeftError::InvalidTree
        );
        self.tree_shard.set_inner(TreeShard {
            merkle_tree: self.merkle_tree.key(),
            index,
            minted: 0,
            capacity: 1u64 << max_depth,
            full: false,
            bump,
        });
        self.registry.active_tree = self.merkle_tree.key();
        self.registry.tree_count = self
            .registry
            .tree_count
            .checked_add(1)
            .ok_or(WeftError::MathOverflow)?;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct RegisterNode<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,
    #[account(mut, seeds = [REGISTRY_SEED], bump = registry.bump, constraint = !registry.paused @ WeftError::Paused)]
    pub registry: Account<'info, Registry>,
    #[account(
        mut,
        seeds = [TREE_SEED, &tree_shard.index.to_le_bytes()],
        bump = tree_shard.bump,
        constraint = tree_shard.merkle_tree == registry.active_tree @ WeftError::InvalidTree,
        constraint = !tree_shard.full @ WeftError::TreeFull,
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
    #[account(mut, address = registry.active_tree @ WeftError::InvalidTree)]
    pub merkle_tree: UncheckedAccount<'info>,
    /// CHECK: the registry's MPL-Core collection.
    #[account(mut, address = registry.collection @ WeftError::InvalidCollection)]
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

impl RegisterNode<'_> {
    #[allow(clippy::too_many_arguments)]
    fn register_node(
        &mut self,
        node_id: u64,
        geo: u32,
        capabilities: u32,
        endpoint_hash: [u8; 32],
        availability: u8,
        metadata_uri: String,
        bump: u8,
    ) -> Result<()> {
        require!(geo_is_valid(geo), WeftError::InvalidGeo);
        require!(
            capabilities_valid(capabilities),
            WeftError::InvalidCapabilities
        );
        require!(
            availability_is_valid(availability),
            WeftError::InvalidAvailability
        );
        let now = Clock::get()?.unix_timestamp;
        let merkle_tree = self.registry.active_tree;
        let leaf_nonce = self.tree_shard.minted;
        let asset_id = get_asset_id(&merkle_tree, leaf_nonce);
        let sequence = self
            .registry
            .node_sequence
            .checked_add(1)
            .ok_or(WeftError::MathOverflow)?;
        self.registry.node_sequence = sequence;
        self.registry.node_count = self
            .registry
            .node_count
            .checked_add(1)
            .ok_or(WeftError::MathOverflow)?;
        self.node.set_inner(NodeState {
            operator: self.operator.key(),
            node_id,
            geo,
            capabilities,
            endpoint_hash,
            availability,
            status: STATUS_ACTIVE,
            registered_at: now,
            updated_at: now,
            reputation: BPS as u16,
            stake_amount: 0,
            bump,
            sequence,
            asset_id,
            merkle_tree,
            leaf_nonce,
        });

        // Mint the node's ownership/identity cNFT into the registry collection,
        // signed by the registry PDA (tree delegate + collection authority).
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

        self.tree_shard.minted = self
            .tree_shard
            .minted
            .checked_add(1)
            .ok_or(WeftError::MathOverflow)?;
        if self.tree_shard.minted >= self.tree_shard.capacity {
            self.tree_shard.full = true;
        }
        Ok(())
    }
}

#[derive(Accounts)]
pub struct UpdateNode<'info> {
    pub operator: Signer<'info>,
    #[account(
        mut,
        seeds = [NODE_SEED, operator.key().as_ref(), &node.node_id.to_le_bytes()],
        bump = node.bump,
        has_one = operator @ WeftError::Unauthorized,
    )]
    pub node: Account<'info, NodeState>,
}

impl UpdateNode<'_> {
    fn update_node(
        &mut self,
        geo: Option<u32>,
        capabilities: Option<u32>,
        endpoint_hash: Option<[u8; 32]>,
        availability: Option<u8>,
    ) -> Result<()> {
        if let Some(v) = geo {
            require!(geo_is_valid(v), WeftError::InvalidGeo);
            self.node.geo = v;
        }
        if let Some(v) = capabilities {
            require!(capabilities_valid(v), WeftError::InvalidCapabilities);
            self.node.capabilities = v;
        }
        if let Some(v) = endpoint_hash {
            self.node.endpoint_hash = v;
        }
        if let Some(v) = availability {
            require!(availability_is_valid(v), WeftError::InvalidAvailability);
            self.node.availability = v;
        }
        self.node.updated_at = Clock::get()?.unix_timestamp;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct DeregisterNode<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,
    #[account(mut, seeds = [REGISTRY_SEED], bump = registry.bump)]
    pub registry: Account<'info, Registry>,
    #[account(
        mut,
        seeds = [NODE_SEED, operator.key().as_ref(), &node.node_id.to_le_bytes()],
        bump = node.bump,
        has_one = operator @ WeftError::Unauthorized,
        close = operator,
    )]
    pub node: Account<'info, NodeState>,
}

impl DeregisterNode<'_> {
    fn deregister_node(&mut self) -> Result<()> {
        // Closing the NodeState PDA would strand any still-staked tokens: WithdrawUnstaked
        // requires this same PDA by seed, so a node deregistered with a live stake position
        // could never withdraw again. Require the stake fully wound down first
        // (request_unstake → unbonding → withdraw_unstaked leaves node.stake_amount == 0).
        require!(self.node.stake_amount == 0, WeftError::StakeNotWithdrawn);
        self.registry.node_count = self.registry.node_count.saturating_sub(1);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct CoreAdmin<'info> {
    pub authority: Signer<'info>,
    #[account(mut, seeds = [REGISTRY_SEED], bump = registry.bump, constraint = registry.authority == authority.key() @ WeftError::Unauthorized)]
    pub registry: Account<'info, Registry>,
    #[account(mut, seeds = [STAKING_CONFIG_SEED], bump = staking_config.bump, constraint = staking_config.authority == authority.key() @ WeftError::Unauthorized)]
    pub staking_config: Account<'info, StakingConfig>,
    #[account(mut, seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, constraint = distributor.authority == authority.key() @ WeftError::Unauthorized)]
    pub distributor: Account<'info, Distributor>,
}

impl CoreAdmin<'_> {
    fn set_core_authority(&mut self, new_authority: Pubkey) -> Result<()> {
        self.registry.authority = new_authority;
        self.staking_config.authority = new_authority;
        self.distributor.authority = new_authority;
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<()> {
        self.registry.paused = paused;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct Stake<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,
    #[account(seeds = [STAKING_CONFIG_SEED], bump = staking_config.bump, has_one = mint)]
    pub staking_config: Account<'info, StakingConfig>,
    #[account(
        mut,
        seeds = [NODE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump = node.bump,
        has_one = operator @ WeftError::Unauthorized,
    )]
    pub node: Account<'info, NodeState>,
    #[account(
        init_if_needed,
        payer = operator,
        space = 8 + StakePosition::INIT_SPACE,
        seeds = [STAKE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump
    )]
    pub position: Account<'info, StakePosition>,
    #[account(
        init_if_needed,
        payer = operator,
        token::mint = mint,
        token::authority = position,
        seeds = [STAKE_VAULT_SEED, position.key().as_ref()],
        bump
    )]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, token::mint = mint, token::authority = operator)]
    pub operator_token_account: InterfaceAccount<'info, TokenAccount>,
    pub mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl Stake<'_> {
    fn stake(&mut self, node_id: u64, amount: u64, lock_duration: i64, bump: u8) -> Result<()> {
        require!(amount > 0, WeftError::ZeroAmount);
        require!(
            (MIN_LOCK_SECONDS..=MAX_LOCK_SECONDS).contains(&lock_duration),
            WeftError::InvalidLock
        );
        let now = Clock::get()?.unix_timestamp;
        self.position.operator = self.operator.key();
        self.position.node_id = node_id;
        self.position.mint = self.mint.key();
        self.position.vault = self.vault.key();
        self.position.bump = bump;
        self.position.amount = self
            .position
            .amount
            .checked_add(amount)
            .ok_or(WeftError::MathOverflow)?;
        let new_lock = now
            .checked_add(lock_duration)
            .ok_or(WeftError::MathOverflow)?;
        if new_lock > self.position.locked_until {
            self.position.locked_until = new_lock;
        }
        transfer_checked(
            CpiContext::new(
                self.token_program.key(),
                TransferChecked {
                    from: self.operator_token_account.to_account_info(),
                    mint: self.mint.to_account_info(),
                    to: self.vault.to_account_info(),
                    authority: self.operator.to_account_info(),
                },
            ),
            amount,
            self.mint.decimals,
        )?;
        self.node.stake_amount = self.position.amount;
        self.node.updated_at = now;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct RequestUnstake<'info> {
    pub operator: Signer<'info>,
    #[account(
        mut,
        seeds = [STAKE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump = position.bump,
        constraint = position.operator == operator.key() @ WeftError::Unauthorized,
    )]
    pub position: Account<'info, StakePosition>,
    #[account(seeds = [STAKING_CONFIG_SEED], bump = staking_config.bump)]
    pub staking_config: Account<'info, StakingConfig>,
}

impl RequestUnstake<'_> {
    fn request_unstake(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, WeftError::ZeroAmount);
        let now = Clock::get()?.unix_timestamp;
        require!(now >= self.position.locked_until, WeftError::Locked);
        let available = self
            .position
            .amount
            .checked_sub(self.position.unbonding_amount)
            .ok_or(WeftError::MathOverflow)?;
        require!(amount <= available, WeftError::InsufficientStake);
        self.position.unbonding_amount = self
            .position
            .unbonding_amount
            .checked_add(amount)
            .ok_or(WeftError::MathOverflow)?;
        self.position.unbonding_until = now
            .checked_add(self.staking_config.unbonding_seconds)
            .ok_or(WeftError::MathOverflow)?;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct WithdrawUnstaked<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,
    #[account(
        mut,
        seeds = [NODE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump = node.bump,
        has_one = operator @ WeftError::Unauthorized,
    )]
    pub node: Account<'info, NodeState>,
    #[account(
        mut,
        seeds = [STAKE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump = position.bump,
        constraint = position.operator == operator.key() @ WeftError::Unauthorized,
        has_one = vault,
    )]
    pub position: Account<'info, StakePosition>,
    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, token::mint = mint, token::authority = operator)]
    pub operator_token_account: InterfaceAccount<'info, TokenAccount>,
    pub mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl WithdrawUnstaked<'_> {
    fn withdraw_unstaked(&mut self, node_id: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let amount = self.position.unbonding_amount;
        require!(amount > 0, WeftError::ZeroAmount);
        require!(
            now >= self.position.unbonding_until,
            WeftError::StillUnbonding
        );
        let id = node_id.to_le_bytes();
        let bump = self.position.bump;
        let seeds: &[&[&[u8]]] = &[&[STAKE_SEED, self.operator.key.as_ref(), &id, &[bump]]];
        transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.key(),
                TransferChecked {
                    from: self.vault.to_account_info(),
                    mint: self.mint.to_account_info(),
                    to: self.operator_token_account.to_account_info(),
                    authority: self.position.to_account_info(),
                },
                seeds,
            ),
            amount,
            self.mint.decimals,
        )?;
        self.position.amount = self
            .position
            .amount
            .checked_sub(amount)
            .ok_or(WeftError::MathOverflow)?;
        self.position.unbonding_amount = 0;
        self.node.stake_amount = self.position.amount;
        self.node.updated_at = now;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(node_id: u64)]
pub struct CloseEmptyStakePosition<'info> {
    #[account(mut)]
    pub operator: Signer<'info>,
    #[account(
        mut,
        seeds = [STAKE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()],
        bump = position.bump,
        constraint = position.operator == operator.key() @ WeftError::Unauthorized,
        has_one = vault,
        close = operator,
    )]
    pub position: Account<'info, StakePosition>,
    #[account(mut, token::authority = position)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl CloseEmptyStakePosition<'_> {
    fn close_empty_stake_position(&mut self, node_id: u64) -> Result<()> {
        require!(self.position.amount == 0, WeftError::AccountNotEmpty);
        require!(
            self.position.unbonding_amount == 0,
            WeftError::AccountNotEmpty
        );
        require!(self.vault.amount == 0, WeftError::AccountNotEmpty);
        let id = node_id.to_le_bytes();
        let bump = self.position.bump;
        let seeds: &[&[&[u8]]] = &[&[STAKE_SEED, self.operator.key.as_ref(), &id, &[bump]]];
        close_token_account_ctx(
            self.token_program.key(),
            self.vault.to_account_info(),
            self.operator.to_account_info(),
            self.position.to_account_info(),
            seeds,
        )
    }
}

#[derive(Accounts)]
pub struct DepositEscrow<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, has_one = reward_mint)]
    pub distributor: Account<'info, Distributor>,
    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + PaymentEscrow::INIT_SPACE,
        seeds = [ESCROW_SEED, owner.key().as_ref()],
        bump
    )]
    pub escrow: Account<'info, PaymentEscrow>,
    #[account(
        init_if_needed,
        payer = owner,
        token::mint = reward_mint,
        token::authority = escrow,
        seeds = [ESCROW_VAULT_SEED, owner.key().as_ref()],
        bump
    )]
    pub escrow_vault: InterfaceAccount<'info, TokenAccount>,
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = reward_mint, token::authority = owner)]
    pub owner_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl DepositEscrow<'_> {
    fn deposit_escrow(&mut self, amount: u64, escrow_bump: u8) -> Result<()> {
        require!(amount > 0, WeftError::ZeroAmount);
        init_or_validate_escrow(
            &mut self.escrow,
            self.owner.key(),
            self.reward_mint.key(),
            self.escrow_vault.key(),
            escrow_bump,
        )?;
        self.escrow.balance = self
            .escrow
            .balance
            .checked_add(amount)
            .ok_or(WeftError::MathOverflow)?;
        self.escrow.total_deposited = self
            .escrow
            .total_deposited
            .checked_add(amount)
            .ok_or(WeftError::MathOverflow)?;
        transfer_checked(
            CpiContext::new(
                self.token_program.key(),
                TransferChecked {
                    from: self.owner_token_account.to_account_info(),
                    mint: self.reward_mint.to_account_info(),
                    to: self.escrow_vault.to_account_info(),
                    authority: self.owner.to_account_info(),
                },
            ),
            amount,
            self.reward_mint.decimals,
        )
    }
}

#[derive(Accounts)]
pub struct WithdrawEscrow<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, owner.key().as_ref()],
        bump = escrow.bump,
        constraint = escrow.owner == owner.key() @ WeftError::Unauthorized,
        constraint = escrow.mint == reward_mint.key() @ WeftError::InvalidEscrow,
        constraint = escrow.vault == escrow_vault.key() @ WeftError::InvalidEscrow,
    )]
    pub escrow: Account<'info, PaymentEscrow>,
    #[account(mut, token::mint = reward_mint, token::authority = escrow)]
    pub escrow_vault: InterfaceAccount<'info, TokenAccount>,
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = reward_mint, token::authority = owner)]
    pub owner_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl WithdrawEscrow<'_> {
    fn withdraw_escrow(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, WeftError::ZeroAmount);
        require!(
            self.escrow.balance >= amount,
            WeftError::InsufficientEscrow
        );
        self.escrow.balance = self
            .escrow
            .balance
            .checked_sub(amount)
            .ok_or(WeftError::MathOverflow)?;
        let owner = self.owner.key();
        let seeds: &[&[&[u8]]] = &[&[ESCROW_SEED, owner.as_ref(), &[self.escrow.bump]]];
        transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.key(),
                TransferChecked {
                    from: self.escrow_vault.to_account_info(),
                    mint: self.reward_mint.to_account_info(),
                    to: self.owner_token_account.to_account_info(),
                    authority: self.escrow.to_account_info(),
                },
                seeds,
            ),
            amount,
            self.reward_mint.decimals,
        )
    }
}

#[derive(Accounts)]
pub struct CloseEscrow<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, owner.key().as_ref()],
        bump = escrow.bump,
        constraint = escrow.owner == owner.key() @ WeftError::Unauthorized,
        constraint = escrow.vault == escrow_vault.key() @ WeftError::InvalidEscrow,
        close = owner,
    )]
    pub escrow: Account<'info, PaymentEscrow>,
    #[account(mut, token::authority = escrow)]
    pub escrow_vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl CloseEscrow<'_> {
    fn close_escrow(&mut self) -> Result<()> {
        require!(self.escrow.balance == 0, WeftError::AccountNotEmpty);
        require!(self.escrow_vault.amount == 0, WeftError::AccountNotEmpty);
        let owner = self.owner.key();
        let seeds: &[&[&[u8]]] = &[&[ESCROW_SEED, owner.as_ref(), &[self.escrow.bump]]];
        close_token_account_ctx(
            self.token_program.key(),
            self.escrow_vault.to_account_info(),
            self.owner.to_account_info(),
            self.escrow.to_account_info(),
            seeds,
        )
    }
}

#[derive(Accounts)]
pub struct PayTraffic<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, has_one = reward_mint, has_one = reward_vault, has_one = treasury)]
    pub distributor: Account<'info, Distributor>,
    #[account(mut)]
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = reward_mint, token::authority = payer)]
    pub payer_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub treasury: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl PayTraffic<'_> {
    fn pay_traffic(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, WeftError::ZeroAmount);
        split_from_user(
            amount,
            self.reward_mint.decimals,
            self.token_program.key(),
            self.payer_token_account.to_account_info(),
            self.reward_mint.to_account_info(),
            self.reward_vault.to_account_info(),
            self.treasury.to_account_info(),
            self.payer.to_account_info(),
            None,
        )
    }
}

#[derive(Accounts)]
pub struct PayTrafficFromEscrow<'info> {
    pub owner: Signer<'info>,
    #[account(seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, has_one = reward_mint, has_one = reward_vault, has_one = treasury)]
    pub distributor: Account<'info, Distributor>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, owner.key().as_ref()],
        bump = escrow.bump,
        constraint = escrow.owner == owner.key() @ WeftError::Unauthorized,
        constraint = escrow.mint == reward_mint.key() @ WeftError::InvalidEscrow,
        constraint = escrow.vault == escrow_vault.key() @ WeftError::InvalidEscrow,
    )]
    pub escrow: Account<'info, PaymentEscrow>,
    #[account(mut, token::mint = reward_mint, token::authority = escrow)]
    pub escrow_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub treasury: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl PayTrafficFromEscrow<'_> {
    fn pay_traffic_from_escrow(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, WeftError::ZeroAmount);
        require!(
            self.escrow.balance >= amount,
            WeftError::InsufficientEscrow
        );
        self.escrow.balance = self
            .escrow
            .balance
            .checked_sub(amount)
            .ok_or(WeftError::MathOverflow)?;
        self.escrow.total_spent = self
            .escrow
            .total_spent
            .checked_add(amount)
            .ok_or(WeftError::MathOverflow)?;
        let owner = self.owner.key();
        let seeds: &[&[&[u8]]] = &[&[ESCROW_SEED, owner.as_ref(), &[self.escrow.bump]]];
        split_from_user(
            amount,
            self.reward_mint.decimals,
            self.token_program.key(),
            self.escrow_vault.to_account_info(),
            self.reward_mint.to_account_info(),
            self.reward_vault.to_account_info(),
            self.treasury.to_account_info(),
            self.escrow.to_account_info(),
            Some(seeds),
        )
    }
}

#[derive(Accounts)]
pub struct SettleFromEscrow<'info> {
    /// The settlement authority (relay/aggregator) — bills metered usage on the user's behalf. Must
    /// equal `distributor.poster_authority`; it can only move escrow funds into the protocol splits,
    /// never to an arbitrary account, and never more than the escrow's own balance.
    pub settle_authority: Signer<'info>,
    #[account(
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        has_one = reward_mint,
        has_one = reward_vault,
        has_one = treasury,
        constraint = distributor.poster_authority == settle_authority.key() @ WeftError::Unauthorized,
    )]
    pub distributor: Account<'info, Distributor>,
    #[account(
        mut,
        seeds = [ESCROW_SEED, escrow.owner.as_ref()],
        bump = escrow.bump,
        constraint = escrow.mint == reward_mint.key() @ WeftError::InvalidEscrow,
        constraint = escrow.vault == escrow_vault.key() @ WeftError::InvalidEscrow,
    )]
    pub escrow: Account<'info, PaymentEscrow>,
    #[account(mut, token::mint = reward_mint, token::authority = escrow)]
    pub escrow_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub treasury: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl SettleFromEscrow<'_> {
    fn settle_from_escrow(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, WeftError::ZeroAmount);
        require!(self.escrow.balance >= amount, WeftError::InsufficientEscrow);
        self.escrow.balance = self
            .escrow
            .balance
            .checked_sub(amount)
            .ok_or(WeftError::MathOverflow)?;
        self.escrow.total_spent = self
            .escrow
            .total_spent
            .checked_add(amount)
            .ok_or(WeftError::MathOverflow)?;
        let owner = self.escrow.owner;
        let seeds: &[&[&[u8]]] = &[&[ESCROW_SEED, owner.as_ref(), &[self.escrow.bump]]];
        split_from_user(
            amount,
            self.reward_mint.decimals,
            self.token_program.key(),
            self.escrow_vault.to_account_info(),
            self.reward_mint.to_account_info(),
            self.reward_vault.to_account_info(),
            self.treasury.to_account_info(),
            self.escrow.to_account_info(),
            Some(seeds),
        )
    }
}

#[derive(Accounts)]
pub struct FundRewardVault<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    #[account(seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, has_one = reward_mint, has_one = reward_vault)]
    pub distributor: Account<'info, Distributor>,
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut, token::mint = reward_mint, token::authority = funder)]
    pub funder_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl FundRewardVault<'_> {
    fn fund_reward_vault(&mut self, amount: u64) -> Result<()> {
        require!(amount > 0, WeftError::ZeroAmount);
        transfer_checked(
            CpiContext::new(
                self.token_program.key(),
                TransferChecked {
                    from: self.funder_token_account.to_account_info(),
                    mint: self.reward_mint.to_account_info(),
                    to: self.reward_vault.to_account_info(),
                    authority: self.funder.to_account_info(),
                },
            ),
            amount,
            self.reward_mint.decimals,
        )
    }
}

#[derive(Accounts)]
#[instruction(epoch: u64)]
pub struct PostEpoch<'info> {
    #[account(mut)]
    pub poster: Signer<'info>,
    #[account(
        mut,
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        has_one = reward_vault,
        constraint = distributor.poster_authority == poster.key() @ WeftError::Unauthorized,
    )]
    pub distributor: Account<'info, Distributor>,
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(init, payer = poster, space = 8 + EpochDistribution::INIT_SPACE, seeds = [EPOCH_SEED, &epoch.to_le_bytes()], bump)]
    pub epoch_distribution: Account<'info, EpochDistribution>,
    pub system_program: Program<'info, System>,
}

impl PostEpoch<'_> {
    fn post_epoch(
        &mut self,
        epoch: u64,
        merkle_root: [u8; 32],
        total_reward: u64,
        num_nodes: u32,
        bump: u8,
    ) -> Result<()> {
        require!(
            epoch > self.distributor.current_epoch,
            WeftError::NonMonotonicEpoch
        );
        let outstanding = self
            .distributor
            .cumulative_obligated
            .checked_sub(self.distributor.cumulative_claimed)
            .ok_or(WeftError::MathOverflow)?;
        let needed = outstanding
            .checked_add(total_reward)
            .ok_or(WeftError::MathOverflow)?;
        require!(
            self.reward_vault.amount >= needed,
            WeftError::InsufficientVault
        );
        self.epoch_distribution.set_inner(EpochDistribution {
            epoch,
            merkle_root,
            total_reward,
            total_claimed: 0,
            num_nodes,
            posted_at: Clock::get()?.unix_timestamp,
            swept: false,
            bump,
        });
        self.distributor.cumulative_obligated = self
            .distributor
            .cumulative_obligated
            .checked_add(total_reward)
            .ok_or(WeftError::MathOverflow)?;
        self.distributor.current_epoch = epoch;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(epoch: u64, node_id: u64)]
pub struct Claim<'info> {
    #[account(mut)]
    pub claimant: Signer<'info>,
    #[account(mut, seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, has_one = reward_mint, has_one = reward_vault)]
    pub distributor: Account<'info, Distributor>,
    #[account(mut, seeds = [EPOCH_SEED, &epoch.to_le_bytes()], bump = epoch_distribution.bump)]
    pub epoch_distribution: Account<'info, EpochDistribution>,
    /// CHECK: bound into the merkle leaf and token account authority.
    pub operator: UncheckedAccount<'info>,
    #[account(init, payer = claimant, space = 8 + ClaimStatus::INIT_SPACE, seeds = [CLAIM_SEED, &epoch.to_le_bytes(), operator.key().as_ref(), &node_id.to_le_bytes()], bump)]
    pub claim_status: Account<'info, ClaimStatus>,
    pub reward_mint: InterfaceAccount<'info, Mint>,
    #[account(mut)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, token::mint = reward_mint, token::authority = operator)]
    pub operator_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl Claim<'_> {
    fn claim(
        &mut self,
        epoch: u64,
        node_id: u64,
        amount: u64,
        proof: Vec<[u8; 32]>,
        bump: u8,
    ) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(
            now >= self
                .epoch_distribution
                .posted_at
                .checked_add(self.distributor.dispute_window_seconds)
                .ok_or(WeftError::MathOverflow)?,
            WeftError::DisputeWindowOpen
        );
        let leaf = hash_reward_leaf(epoch, &self.operator.key().to_bytes(), node_id, amount);
        require!(
            merkle_verify(&proof, self.epoch_distribution.merkle_root, leaf),
            WeftError::InvalidProof
        );
        self.epoch_distribution.total_claimed = self
            .epoch_distribution
            .total_claimed
            .checked_add(amount)
            .ok_or(WeftError::MathOverflow)?;
        require!(
            self.epoch_distribution.total_claimed <= self.epoch_distribution.total_reward,
            WeftError::EpochOverclaim
        );
        self.distributor.cumulative_claimed = self
            .distributor
            .cumulative_claimed
            .checked_add(amount)
            .ok_or(WeftError::MathOverflow)?;
        self.claim_status.set_inner(ClaimStatus {
            epoch,
            operator: self.operator.key(),
            node_id,
            amount,
            claimed_at: now,
            disputed: false,
            bump,
        });
        let seeds: &[&[&[u8]]] = &[&[DISTRIBUTOR_SEED, &[self.distributor.bump]]];
        transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.key(),
                TransferChecked {
                    from: self.reward_vault.to_account_info(),
                    mint: self.reward_mint.to_account_info(),
                    to: self.operator_token_account.to_account_info(),
                    authority: self.distributor.to_account_info(),
                },
                seeds,
            ),
            amount,
            self.reward_mint.decimals,
        )
    }
}

#[derive(Accounts)]
#[instruction(epoch: u64, node_id: u64)]
pub struct Dispute<'info> {
    #[account(mut)]
    pub dispute_authority: Signer<'info>,
    #[account(seeds = [DISTRIBUTOR_SEED], bump = distributor.bump, constraint = distributor.dispute_authority == dispute_authority.key() @ WeftError::Unauthorized)]
    pub distributor: Box<Account<'info, Distributor>>,
    #[account(seeds = [EPOCH_SEED, &epoch.to_le_bytes()], bump = epoch_distribution.bump)]
    pub epoch_distribution: Box<Account<'info, EpochDistribution>>,
    /// CHECK: bound into the disputed claim PDA.
    pub operator: UncheckedAccount<'info>,
    #[account(init, payer = dispute_authority, space = 8 + ClaimStatus::INIT_SPACE, seeds = [CLAIM_SEED, &epoch.to_le_bytes(), operator.key().as_ref(), &node_id.to_le_bytes()], bump)]
    pub claim_status: Box<Account<'info, ClaimStatus>>,
    #[account(mut, seeds = [NODE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()], bump = node.bump)]
    pub node: Box<Account<'info, NodeState>>,
    #[account(seeds = [STAKING_CONFIG_SEED], bump = staking_config.bump, constraint = staking_config.slash_authority == dispute_authority.key() @ WeftError::Unauthorized, has_one = treasury)]
    pub staking_config: Box<Account<'info, StakingConfig>>,
    #[account(mut, seeds = [STAKE_SEED, operator.key().as_ref(), &node_id.to_le_bytes()], bump = position.bump, has_one = vault)]
    pub position: Box<Account<'info, StakePosition>>,
    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut)]
    pub treasury: InterfaceAccount<'info, TokenAccount>,
    pub stake_mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl Dispute<'_> {
    fn dispute(
        &mut self,
        epoch: u64,
        node_id: u64,
        amount: u64,
        severity_bps: u32,
        slash_amount: u64,
        bump: u8,
    ) -> Result<()> {
        self.claim_status.set_inner(ClaimStatus {
            epoch,
            operator: self.operator.key(),
            node_id,
            amount,
            claimed_at: Clock::get()?.unix_timestamp,
            disputed: true,
            bump,
        });
        if slash_amount > 0 {
            // Bound every seizure to MAX_SLASH_BPS of the position so one dispute can never take
            // the whole stake; the reputational penalty below is the soft-first signal.
            let max_slash = ((self.position.amount as u128)
                .saturating_mul(MAX_SLASH_BPS as u128)
                / (BPS as u128)) as u64;
            let slashed = slash_amount.min(self.position.amount).min(max_slash);
            if slashed > 0 {
                let id = node_id.to_le_bytes();
                let seeds: &[&[&[u8]]] = &[&[
                    STAKE_SEED,
                    self.operator.key.as_ref(),
                    &id,
                    &[self.position.bump],
                ]];
                transfer_checked(
                    CpiContext::new_with_signer(
                        self.token_program.key(),
                        TransferChecked {
                            from: self.vault.to_account_info(),
                            mint: self.stake_mint.to_account_info(),
                            to: self.treasury.to_account_info(),
                            authority: self.position.to_account_info(),
                        },
                        seeds,
                    ),
                    slashed,
                    self.stake_mint.decimals,
                )?;
                self.position.amount = self.position.amount.saturating_sub(slashed);
                if self.position.unbonding_amount > self.position.amount {
                    self.position.unbonding_amount = self.position.amount;
                }
                self.node.stake_amount = self.position.amount;
            }
        }
        if severity_bps > 0 {
            let severity = severity_bps.min(BPS);
            let current = self.node.reputation as u32;
            let penalized = current.saturating_sub(severity / 2).max(REPUTATION_MIN_BPS);
            self.node.reputation = penalized as u16;
        }
        self.node.updated_at = Clock::get()?.unix_timestamp;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct ShutdownCore<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [REGISTRY_SEED],
        bump = registry.bump,
        constraint = registry.authority == authority.key() @ WeftError::Unauthorized,
        close = authority,
    )]
    pub registry: Account<'info, Registry>,
    #[account(
        mut,
        seeds = [STAKING_CONFIG_SEED],
        bump = staking_config.bump,
        constraint = staking_config.authority == authority.key() @ WeftError::Unauthorized,
        close = authority,
    )]
    pub staking_config: Account<'info, StakingConfig>,
    #[account(
        mut,
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        constraint = distributor.authority == authority.key() @ WeftError::Unauthorized,
        has_one = reward_vault,
        close = authority,
    )]
    pub distributor: Account<'info, Distributor>,
    #[account(mut, token::authority = distributor)]
    pub reward_vault: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl ShutdownCore<'_> {
    fn shutdown_core(&mut self) -> Result<()> {
        require!(self.registry.paused, WeftError::ShutdownRequiresPaused);
        require!(self.registry.node_count == 0, WeftError::ShutdownBlocked);
        require!(
            self.distributor.cumulative_obligated == self.distributor.cumulative_claimed,
            WeftError::ShutdownBlocked
        );
        require!(self.reward_vault.amount == 0, WeftError::AccountNotEmpty);
        let seeds: &[&[&[u8]]] = &[&[DISTRIBUTOR_SEED, &[self.distributor.bump]]];
        close_token_account_ctx(
            self.token_program.key(),
            self.reward_vault.to_account_info(),
            self.authority.to_account_info(),
            self.distributor.to_account_info(),
            seeds,
        )
    }
}

fn init_or_validate_escrow(
    escrow: &mut PaymentEscrow,
    owner: Pubkey,
    mint: Pubkey,
    vault: Pubkey,
    bump: u8,
) -> Result<()> {
    if escrow.owner == Pubkey::default() {
        escrow.owner = owner;
        escrow.mint = mint;
        escrow.vault = vault;
        escrow.bump = bump;
        return Ok(());
    }
    require!(escrow.owner == owner, WeftError::Unauthorized);
    require!(escrow.mint == mint, WeftError::InvalidEscrow);
    require!(escrow.vault == vault, WeftError::InvalidEscrow);
    Ok(())
}

fn close_token_account_ctx<'info>(
    token_program: Pubkey,
    account: AccountInfo<'info>,
    destination: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    let ix = spl_token_interface::instruction::close_account(
        &token_program,
        account.key,
        destination.key,
        authority.key,
        &[],
    )?;
    anchor_lang::solana_program::program::invoke_signed(
        &ix,
        &[account, destination, authority],
        signer_seeds,
    )
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn split_from_user<'info>(
    amount: u64,
    decimals: u8,
    token_program: Pubkey,
    source: AccountInfo<'info>,
    mint: AccountInfo<'info>,
    reward_vault: AccountInfo<'info>,
    treasury: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    signer: Option<&[&[&[u8]]]>,
) -> Result<()> {
    let split = split_payment(amount);
    if split.nodes > 0 {
        transfer_checked_ctx(
            token_program,
            source.clone(),
            mint.clone(),
            reward_vault,
            authority.clone(),
            signer,
            split.nodes,
            decimals,
        )?;
    }
    if split.treasury > 0 {
        transfer_checked_ctx(
            token_program,
            source.clone(),
            mint.clone(),
            treasury,
            authority.clone(),
            signer,
            split.treasury,
            decimals,
        )?;
    }
    if split.burn > 0 {
        let ctx = CpiContext::new(
            token_program,
            BurnChecked {
                mint,
                from: source,
                authority,
            },
        );
        match signer {
            Some(seeds) => burn_checked(ctx.with_signer(seeds), split.burn, decimals)?,
            None => burn_checked(ctx, split.burn, decimals)?,
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn transfer_checked_ctx<'info>(
    token_program: Pubkey,
    from: AccountInfo<'info>,
    mint: AccountInfo<'info>,
    to: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    signer: Option<&[&[&[u8]]]>,
    amount: u64,
    decimals: u8,
) -> Result<()> {
    let ctx = CpiContext::new(
        token_program,
        TransferChecked {
            from,
            mint,
            to,
            authority,
        },
    );
    match signer {
        Some(seeds) => transfer_checked(ctx.with_signer(seeds), amount, decimals)?,
        None => transfer_checked(ctx, amount, decimals)?,
    }
    Ok(())
}

#[error_code]
pub enum WeftError {
    #[msg("Caller is not authorized")]
    Unauthorized,
    #[msg("Registry is paused")]
    Paused,
    #[msg("Invalid geo value")]
    InvalidGeo,
    #[msg("Invalid capability flags")]
    InvalidCapabilities,
    #[msg("Invalid availability value")]
    InvalidAvailability,
    #[msg("Invalid lock duration")]
    InvalidLock,
    #[msg("Invalid unbonding duration")]
    InvalidUnbonding,
    #[msg("Invalid settlement window")]
    InvalidWindow,
    #[msg("Stake is still locked")]
    Locked,
    #[msg("Unbonding window has not elapsed")]
    StillUnbonding,
    #[msg("Insufficient staked balance")]
    InsufficientStake,
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Escrow balance is insufficient")]
    InsufficientEscrow,
    #[msg("Escrow account does not match the expected owner, mint, or vault")]
    InvalidEscrow,
    #[msg("Epoch must be strictly increasing")]
    NonMonotonicEpoch,
    #[msg("Reward vault cannot cover the posted obligations")]
    InsufficientVault,
    #[msg("Dispute window has not elapsed")]
    DisputeWindowOpen,
    #[msg("Merkle proof is invalid")]
    InvalidProof,
    #[msg("Epoch over-claimed")]
    EpochOverclaim,
    #[msg("Account must be empty before it can be closed")]
    AccountNotEmpty,
    #[msg("Core must be paused before shutdown")]
    ShutdownRequiresPaused,
    #[msg("Core still has active state and cannot be shut down")]
    ShutdownBlocked,
    #[msg("Tree shard index must equal the current tree count")]
    TreeIndexMismatch,
    #[msg("Invalid merkle tree or depth")]
    InvalidTree,
    #[msg("Active tree shard is full")]
    TreeFull,
    #[msg("Registry collection does not match")]
    InvalidCollection,
    #[msg("Stake must be fully withdrawn before the node can be deregistered")]
    StakeNotWithdrawn,
}
