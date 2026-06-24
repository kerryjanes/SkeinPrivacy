use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke, system_instruction};
use anchor_lang::Discriminator;
use weft_primitives::{
    BASE_RATE_PER_GB, BOOTSTRAP_NODE_LIMIT, BPS, GEO_BONUS_MAX_BPS, REPUTATION_MAX_BPS,
    REPUTATION_MIN_BPS, SPLIT_BURN_BPS, SPLIT_NODES_BPS, SPLIT_TREASURY_BPS, STAKING_BONUS_BPS,
    STAKING_BONUS_THRESHOLD,
};

use crate::{
    constants::*,
    error::GovError,
    state::{GovernanceConfig, ProtocolConfig},
};

/// Size (after the 8-byte discriminator) of the pre-M8 `ProtocolConfig` data — i.e. the
/// offset at which the M8 `bootstrap_*` fields were appended. A config created before M8
/// is exactly `8 + PRE_M8_DATA_LEN` bytes long.
const PRE_M8_DATA_LEN: usize = 32 + 4 + 4 + 4 + 8 + 8 + 8 + 4 + 4 + 4 + 4 + 8 + 1; // = 93

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
            // Cold-start defaults: first 10k nodes, +50%, no end (governance sets the
            // window via a proposal once the TGE date is known).
            bootstrap_node_limit: BOOTSTRAP_NODE_LIMIT,
            bootstrap_bonus_bps: 5_000,
            bootstrap_end_ts: 0,
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
    pub bootstrap_node_limit: u64,
    pub bootstrap_bonus_bps: u32,
    pub bootstrap_end_ts: i64,
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
                && p.bootstrap_bonus_bps <= weft_primitives::BOOTSTRAP_BONUS_MAX_BPS
                && p.dispute_window_seconds >= 0
                && p.clawback_window_seconds >= 0
                && p.bootstrap_end_ts >= 0,
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
        c.bootstrap_node_limit = p.bootstrap_node_limit;
        c.bootstrap_bonus_bps = p.bootstrap_bonus_bps;
        c.bootstrap_end_ts = p.bootstrap_end_ts;
        Ok(())
    }
}

/// One-shot migration for a `ProtocolConfig` created before M8 (which lacks the appended
/// `bootstrap_*` fields). Anchor deserializes `Account<ProtocolConfig>` *before* a realloc
/// runs, so a pre-M8 (shorter) account can't be loaded as the current struct at all — hence
/// this takes the account unchecked, grows it to the current size, and seeds the appended
/// fields with the spec cold-start defaults. Idempotent: a no-op once already at full size.
#[derive(Accounts)]
pub struct MigrateProtocolConfig<'info> {
    /// Pays the rent for the extra bytes.
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: validated by PDA seeds + the on-chain discriminator; deserialized manually
    /// because the stored layout may be the shorter pre-M8 one.
    #[account(mut, seeds = [PROTOCOL_CONFIG_SEED], bump)]
    pub protocol_config: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl MigrateProtocolConfig<'_> {
    pub fn migrate_protocol_config(&mut self) -> Result<()> {
        let acc = self.protocol_config.to_account_info();
        let new_len = 8 + ProtocolConfig::INIT_SPACE as usize;

        // Verify this really is a ProtocolConfig and decide whether work is needed.
        {
            let data = acc.try_borrow_data()?;
            require!(data.len() >= 8, GovError::InvalidParam);
            require!(
                &data[..8] == ProtocolConfig::DISCRIMINATOR,
                GovError::Unauthorized
            );
            if data.len() >= new_len {
                return Ok(()); // already migrated
            }
        }

        // Top up rent for the larger account, then grow it (zero-filling the new bytes).
        let rent = Rent::get()?;
        let needed = rent.minimum_balance(new_len);
        let current = acc.lamports();
        if needed > current {
            invoke(
                &system_instruction::transfer(self.payer.key, acc.key, needed - current),
                &[
                    self.payer.to_account_info(),
                    acc.clone(),
                    self.system_program.to_account_info(),
                ],
            )?;
        }
        acc.resize(new_len)?;

        // Seed the appended cold-start fields (bootstrap_end_ts stays 0 from zero-fill).
        let mut data = acc.try_borrow_mut_data()?;
        let off = 8 + PRE_M8_DATA_LEN;
        data[off..off + 8].copy_from_slice(&BOOTSTRAP_NODE_LIMIT.to_le_bytes());
        data[off + 8..off + 12].copy_from_slice(&5_000u32.to_le_bytes());
        Ok(())
    }
}
