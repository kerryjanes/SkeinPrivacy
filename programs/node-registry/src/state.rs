use anchor_lang::prelude::*;

/// Singleton registry config. PDA `[REGISTRY_SEED]`.
#[account]
#[derive(InitSpace)]
pub struct Registry {
    /// Admin / governance handle (M5 → DAO multisig). Can rotate trees and pause.
    pub authority: Pubkey,
    /// The MPL-Core collection that all node cNFTs belong to.
    pub collection: Pubkey,
    /// The merkle tree new nodes currently mint into.
    pub active_tree: Pubkey,
    /// Number of tree shards provisioned so far.
    pub tree_count: u16,
    /// Currently registered (active) nodes.
    pub node_count: u64,
    /// Program allowed to write `NodeState.reputation` (M3). Defaults to `authority`.
    pub reputation_authority: Pubkey,
    /// Program allowed to write `NodeState.stake_amount` (M3). Defaults to `authority`.
    pub staking_authority: Pubkey,
    /// When true, `register` is rejected.
    pub paused: bool,
    pub bump: u8,
}

/// One merkle-tree shard. PDA `[TREE_SEED, index_le]`.
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

/// The authoritative mutable record for a node. PDA `[NODE_SEED, operator, node_id_le]`.
/// The cNFT (identified by `asset_id`) is the ownership/identity token; this PDA is
/// the cheap-to-update on-chain twin queried by the indexer and reward engine.
#[account]
#[derive(InitSpace)]
pub struct NodeState {
    /// Node owner; also the cNFT `leaf_owner`.
    pub operator: Pubkey,
    /// Operator-scoped disambiguator (PDA seed).
    pub node_id: u64,
    /// The Bubblegum V2 cNFT asset id bound to this node.
    pub asset_id: Pubkey,
    /// Which tree shard holds the leaf.
    pub merkle_tree: Pubkey,
    /// Leaf index / nonce at mint time (to recompute the leaf / fetch proofs).
    pub leaf_nonce: u64,
    /// Packed geohash (see `weft_primitives::GEO_BITS`).
    pub geo: u32,
    /// Capability bitflags (see `weft_primitives::capability`).
    pub capabilities: u32,
    /// Commitment to the off-chain connection endpoint (DHT pointer).
    pub endpoint_hash: [u8; 32],
    /// Self-reported availability percentage (0..=100).
    pub availability: u8,
    /// One of `STATUS_*`.
    pub status: u8,
    pub registered_at: i64,
    pub updated_at: i64,
    /// Reputation in bps (0..=10000), written by the M3 reputation program.
    pub reputation: u16,
    /// Staked $WEFT mirrored by the M3 staking program.
    pub stake_amount: u64,
    pub bump: u8,
}
