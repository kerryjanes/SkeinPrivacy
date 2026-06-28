// False positives from the Anchor `#[program]` macro expansion, not our code.
#![allow(clippy::diverging_sub_expression)]

pub mod constants;
pub mod error;
pub mod external;
pub mod instructions;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

declare_id!("8uywvvcGdANC1WM7g1iuEq3crjBwhy5uP5UReKb3xNUE");

#[program]
pub mod governance {
    use super::*;

    /// Create the DAO config singleton.
    pub fn initialize_governance(
        ctx: Context<InitializeGovernance>,
        default_quorum: u64,
        default_approval_threshold_bps: u32,
        voting_period_seconds: i64,
        execution_delay_seconds: i64,
        min_proposal_stake: u64,
    ) -> Result<()> {
        ctx.accounts.initialize_governance(
            default_quorum,
            default_approval_threshold_bps,
            voting_period_seconds,
            execution_delay_seconds,
            min_proposal_stake,
            ctx.bumps.governance_config,
        )
    }

    /// Re-point the DAO admin (e.g. to the governance authority PDA after bootstrap).
    pub fn set_governance_authority(
        ctx: Context<SetGovernanceAuthority>,
        new_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts.set_governance_authority(new_authority)
    }

    /// Seed the governed `ProtocolConfig` from the spec defaults.
    pub fn initialize_protocol_config(
        ctx: Context<InitializeProtocolConfig>,
        config_authority: Pubkey,
        dispute_window_seconds: i64,
        clawback_window_seconds: i64,
    ) -> Result<()> {
        ctx.accounts.initialize_protocol_config(
            config_authority,
            dispute_window_seconds,
            clawback_window_seconds,
            ctx.bumps.protocol_config,
        )
    }

    /// Update the governed parameters (only the `ProtocolConfig.authority` = DAO PDA).
    pub fn update_protocol_config(
        ctx: Context<UpdateProtocolConfig>,
        params: ProtocolParams,
    ) -> Result<()> {
        ctx.accounts.update_protocol_config(params)
    }

    /// One-shot: grow a pre-M8 `ProtocolConfig` to the current layout (adds the
    /// `bootstrap_*` fields). Idempotent; only ever conforms the account to the
    /// program's own struct and seeds the spec cold-start defaults.
    pub fn migrate_protocol_config(ctx: Context<MigrateProtocolConfig>) -> Result<()> {
        ctx.accounts.migrate_protocol_config()
    }

    /// Open a proposal (proposer must present a stake position ≥ `min_proposal_stake`).
    pub fn create_proposal(ctx: Context<CreateProposal>, node_id: u64, name: String) -> Result<()> {
        ctx.accounts
            .create_proposal(node_id, name, ctx.bumps.proposal)
    }

    /// Attach an instruction to a Draft proposal (executed on success).
    pub fn add_transaction(
        ctx: Context<AddTransaction>,
        index: u16,
        program_id: Pubkey,
        accounts: Vec<TxAccountMeta>,
        data: Vec<u8>,
    ) -> Result<()> {
        ctx.accounts.add_transaction(
            index,
            program_id,
            accounts,
            data,
            ctx.bumps.proposal_transaction,
        )
    }

    /// Move a proposal from Draft to Voting, snapshotting quorum/threshold/window.
    pub fn activate_proposal(ctx: Context<ActivateProposal>) -> Result<()> {
        ctx.accounts.activate_proposal()
    }

    /// Cast a stake-weighted vote from one position.
    pub fn cast_vote(ctx: Context<CastVote>, node_id: u64, vote: VoteKind) -> Result<()> {
        ctx.accounts.cast_vote(node_id, vote, ctx.bumps.vote_record)
    }

    /// Resolve a proposal after voting ends (Succeeded/Defeated).
    pub fn finalize_proposal(ctx: Context<FinalizeProposal>) -> Result<()> {
        ctx.accounts.finalize_proposal()
    }

    /// Execute one attached instruction after the timelock, signed by the DAO PDA.
    pub fn execute_transaction<'info>(
        ctx: Context<'info, ExecuteTransaction<'info>>,
    ) -> Result<()> {
        let bump = ctx.bumps.governance_authority;
        ExecuteTransaction::execute_transaction(ctx, bump)
    }

    /// Cancel a proposal before execution.
    pub fn cancel_proposal(ctx: Context<CancelProposal>) -> Result<()> {
        ctx.accounts.cancel_proposal()
    }
}
