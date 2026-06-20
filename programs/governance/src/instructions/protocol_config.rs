use anchor_lang::prelude::*;
use weft_primitives::{
    BASE_RATE_PER_GB, BPS, GEO_BONUS_MAX_BPS, REPUTATION_MAX_BPS, REPUTATION_MIN_BPS,
    SPLIT_BURN_BPS, SPLIT_NODES_BPS, SPLIT_TREASURY_BPS, STAKING_BONUS_BPS,
    STAKING_BONUS_THRESHOLD,
};

use crate::{
    constants::*,
    error::GovError,
    state::{GovernanceConfig, ProtocolConfig},
};

#[derive(Accounts)]
pub struct InitializeProtocolConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        seeds = [GOVERNANCE_CONFIG_SEED],
        bump = governance_config.bump,
        constraint = governance_config.authority == authority.key() @ GovError::Unauthorized,
    )]
    pub governance_config: Account<'info, GovernanceConfig>,
    #[account(
        init,
        payer = authority,
        space = 8 + ProtocolConfig::INIT_SPACE,
        seeds = [PROTOCOL_CONFIG_SEED],
        bump
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,
    pub system_program: Program<'info, System>,
}

impl InitializeProtocolConfig<'_> {
    /// Seed the governed parameters from the `weft-primitives` defaults so the
    /// protocol launches at its spec values; `config_authority` (the DAO PDA after
    /// bootstrap) is the only key that may later `update_protocol_config`.
    pub fn initialize_protocol_config(
        &mut self,
        config_authority: Pubkey,
        dispute_window_seconds: i64,
        clawback_window_seconds: i64,
        bump: u8,
    ) -> Result<()> {
        self.protocol_config.set_inner(ProtocolConfig {
            authority: config_authority,
            split_nodes_bps: SPLIT_NODES_BPS,
            split_burn_bps: SPLIT_BURN_BPS,
            split_treasury_bps: SPLIT_TREASURY_BPS,
            dispute_window_seconds,
            clawback_window_seconds,
            base_rate_per_gb: BASE_RATE_PER_GB,
            geo_bonus_max_bps: GEO_BONUS_MAX_BPS,
            reputation_min_bps: REPUTATION_MIN_BPS,
            reputation_max_bps: REPUTATION_MAX_BPS,
            staking_bonus_bps: STAKING_BONUS_BPS,
            staking_bonus_threshold: STAKING_BONUS_THRESHOLD,
            bump,
        });
        Ok(())
    }
}

/// The full governed parameter set, supplied by a passed proposal. Grouped to keep
/// the instruction signature readable.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct ProtocolParams {
    pub split_nodes_bps: u32,
    pub split_burn_bps: u32,
    pub split_treasury_bps: u32,
    pub dispute_window_seconds: i64,
    pub clawback_window_seconds: i64,
    pub base_rate_per_gb: u64,
    pub geo_bonus_max_bps: u32,
    pub reputation_min_bps: u32,
    pub reputation_max_bps: u32,
    pub staking_bonus_bps: u32,
    pub staking_bonus_threshold: u64,
}

#[derive(Accounts)]
pub struct UpdateProtocolConfig<'info> {
    /// The DAO PDA (via `execute_transaction` `invoke_signed`) or the bootstrap admin.
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
        constraint = protocol_config.authority == authority.key() @ GovError::Unauthorized,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,
}

impl UpdateProtocolConfig<'_> {
    pub fn update_protocol_config(&mut self, p: ProtocolParams) -> Result<()> {
        // The split must conserve the whole payment and the reward multipliers stay
        // in spec-sane ranges (the aggregator additionally clamps).
        require!(
            p.split_nodes_bps + p.split_burn_bps + p.split_treasury_bps == BPS,
            GovError::InvalidSplit
        );
        require!(
            p.reputation_min_bps <= p.reputation_max_bps
                && p.geo_bonus_max_bps <= BPS
                && p.staking_bonus_bps <= BPS
                && p.dispute_window_seconds >= 0
                && p.clawback_window_seconds >= 0,
            GovError::InvalidParam
        );
        let c = &mut self.protocol_config;
        c.split_nodes_bps = p.split_nodes_bps;
        c.split_burn_bps = p.split_burn_bps;
        c.split_treasury_bps = p.split_treasury_bps;
        c.dispute_window_seconds = p.dispute_window_seconds;
        c.clawback_window_seconds = p.clawback_window_seconds;
        c.base_rate_per_gb = p.base_rate_per_gb;
        c.geo_bonus_max_bps = p.geo_bonus_max_bps;
        c.reputation_min_bps = p.reputation_min_bps;
        c.reputation_max_bps = p.reputation_max_bps;
        c.staking_bonus_bps = p.staking_bonus_bps;
        c.staking_bonus_threshold = p.staking_bonus_threshold;
        Ok(())
    }
}
