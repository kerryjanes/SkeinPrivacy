use anchor_lang::prelude::*;

use crate::constants::{MAX_PROPOSAL_NAME, MAX_TX_ACCOUNTS, MAX_TX_DATA};

/// DAO configuration singleton. PDA `[GOVERNANCE_CONFIG_SEED]`.
#[account]
#[derive(InitSpace)]
pub struct GovernanceConfig {
    /// Admin that may seed defaults / be re-pointed to the DAO PDA after bootstrap.
    pub authority: Pubkey,
    /// The $WEFT mint whose staked balance confers voting power.
    pub gov_mint: Pubkey,
    /// Default absolute quorum (base units) snapshotted onto new proposals.
    pub default_quorum: u64,
    /// Default approval threshold (bps of yes/(yes+no)).
    pub default_approval_threshold_bps: u32,
    /// Voting window length (seconds).
    pub voting_period_seconds: i64,
    /// Timelock between success and executability (seconds).
    pub execution_delay_seconds: i64,
    /// Minimum staked balance (base units) a proposer must present.
    pub min_proposal_stake: u64,
    /// Monotonic proposal id counter.
    pub proposal_count: u64,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub enum ProposalState {
    Draft,
    Voting,
    Succeeded,
    Defeated,
    Executed,
    Cancelled,
}

/// A governance proposal. PDA `[PROPOSAL_SEED, id_le]`.
#[account]
#[derive(InitSpace)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Pubkey,
    pub state: ProposalState,
    pub created_at: i64,
    pub voting_starts_at: i64,
    pub voting_ends_at: i64,
    pub execution_eta: i64,
    /// Snapshotted at activation from `GovernanceConfig`.
    pub quorum: u64,
    pub approval_threshold_bps: u32,
    /// Running vote tallies in base units (u128 never overflows vs 1e18 supply).
    pub yes: u128,
    pub no: u128,
    pub abstain: u128,
    pub transaction_count: u16,
    pub executed_count: u16,
    #[max_len(MAX_PROPOSAL_NAME)]
    pub name: String,
    pub bump: u8,
}

/// A single account reference inside a queued instruction.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, InitSpace)]
pub struct TxAccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

/// One instruction attached to a proposal, executed on success. PDA
/// `[PROPOSAL_TX_SEED, proposal, index_le]`.
#[account]
#[derive(InitSpace)]
pub struct ProposalTransaction {
    pub proposal: Pubkey,
    pub index: u16,
    pub program_id: Pubkey,
    #[max_len(MAX_TX_ACCOUNTS)]
    pub accounts: Vec<TxAccountMeta>,
    #[max_len(MAX_TX_DATA)]
    pub data: Vec<u8>,
    pub executed: bool,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum VoteKind {
    Yes,
    No,
    Abstain,
}

/// One vote cast from one stake position. PDA `[VOTE_SEED, proposal, position]`.
/// `init` makes a position votable exactly once per proposal (double-vote guard).
#[account]
#[derive(InitSpace)]
pub struct VoteRecord {
    pub proposal: Pubkey,
    pub position: Pubkey,
    pub operator: Pubkey,
    pub node_id: u64,
    pub weight: u64,
    pub vote: VoteKind,
    pub bump: u8,
}

/// DAO-governed protocol parameters. PDA `[PROTOCOL_CONFIG_SEED]`. `authority` is
/// the governance authority PDA after bootstrap, so only a passed proposal edits it.
#[account]
#[derive(InitSpace)]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    // Enforced on-chain by rewards-settlement `pay_traffic`.
    pub split_nodes_bps: u32,
    pub split_burn_bps: u32,
    pub split_treasury_bps: u32,
    // Settlement timing (read by the aggregator / future settlement reads).
    pub dispute_window_seconds: i64,
    pub clawback_window_seconds: i64,
    // Reward-formula parameters consumed off-chain by the aggregator; on-chain only
    // as the disputable reference of record.
    pub base_rate_per_gb: u64,
    pub geo_bonus_max_bps: u32,
    pub reputation_min_bps: u32,
    pub reputation_max_bps: u32,
    pub staking_bonus_bps: u32,
    pub staking_bonus_threshold: u64,
    pub bump: u8,
    // Cold-start bonus (M8) — consumed off-chain by the aggregator, governed here.
    // Nodes with `NodeState.sequence <= bootstrap_node_limit` earn `bootstrap_bonus_bps`
    // until `bootstrap_end_ts`. Appended after `bump` so the existing on-chain
    // `ProtocolConfig` layout is preserved.
    pub bootstrap_node_limit: u64,
    pub bootstrap_bonus_bps: u32,
    pub bootstrap_end_ts: i64,
}
