use anchor_lang::prelude::*;

use crate::{
    constants::*,
    error::SettlementError,
    external::{invoke_penalize, invoke_slash, NODE_REGISTRY_ID, REPUTATION_ID, STAKING_ID},
    state::{ClaimStatus, Distributor, EpochDistribution},
};

/// Flag a fraudulent epoch leaf and punish the node: the disputed `ClaimStatus`
/// PDA is `init`'d with `disputed = true`, which (being the same PDA `claim`
/// would create) permanently blocks the payout, then the settlement program
/// signs `staking::slash` + `reputation::penalize` as its `[AUTHORITY_SEED]` PDA
/// (wired into `staking.slash_authority` / `reputation.oracle`). Both punishments
/// mirror straight back into `NodeState`.
#[derive(Accounts)]
#[instruction(epoch: u64, node_id: u64)]
pub struct Dispute<'info> {
    #[account(mut)]
    pub dispute_authority: Signer<'info>,
    #[account(
        seeds = [DISTRIBUTOR_SEED],
        bump = distributor.bump,
        constraint = distributor.dispute_authority == dispute_authority.key() @ SettlementError::Unauthorized,
    )]
    pub distributor: Account<'info, Distributor>,
    #[account(seeds = [EPOCH_SEED, &epoch.to_le_bytes()], bump = epoch_distribution.bump)]
    pub epoch_distribution: Account<'info, EpochDistribution>,
    /// CHECK: the disputed leaf's operator; bound into the claim PDA seeds.
    pub operator: UncheckedAccount<'info>,
    #[account(
        init,
        payer = dispute_authority,
        space = 8 + ClaimStatus::INIT_SPACE,
        seeds = [CLAIM_SEED, &epoch.to_le_bytes(), operator.key().as_ref(), &node_id.to_le_bytes()],
        bump
    )]
    pub claim_status: Account<'info, ClaimStatus>,
    /// CHECK: settlement program-signer PDA = staking.slash_authority + reputation.oracle.
    #[account(seeds = [AUTHORITY_SEED], bump)]
    pub program_authority: UncheckedAccount<'info>,

    // ---- staking::slash ----
    /// CHECK: staking program.
    #[account(address = STAKING_ID)]
    pub staking_program: UncheckedAccount<'info>,
    /// CHECK: staking config PDA (validated inside the CPI).
    pub staking_config: UncheckedAccount<'info>,
    /// CHECK: StakePosition PDA (validated inside the CPI).
    #[account(mut)]
    pub staking_position: UncheckedAccount<'info>,
    /// CHECK: staking vault token account (validated inside the CPI).
    #[account(mut)]
    pub staking_vault: UncheckedAccount<'info>,
    /// CHECK: staking treasury token account (validated inside the CPI).
    #[account(mut)]
    pub staking_treasury: UncheckedAccount<'info>,
    /// CHECK: stake mint (validated inside the CPI).
    pub stake_mint: UncheckedAccount<'info>,
    /// CHECK: staking's own [AUTHORITY_SEED] program-signer PDA (for its NodeState mirror).
    pub staking_program_authority: UncheckedAccount<'info>,

    // ---- reputation::penalize ----
    /// CHECK: reputation program.
    #[account(address = REPUTATION_ID)]
    pub reputation_program: UncheckedAccount<'info>,
    /// CHECK: reputation config PDA (validated inside the CPI).
    pub reputation_config: UncheckedAccount<'info>,
    /// CHECK: ReputationState PDA (validated inside the CPI).
    #[account(mut)]
    pub reputation_state: UncheckedAccount<'info>,
    /// CHECK: reputation's own [AUTHORITY_SEED] program-signer PDA (for its NodeState mirror).
    pub reputation_program_authority: UncheckedAccount<'info>,

    // ---- shared mirror targets ----
    /// CHECK: node-registry program.
    #[account(address = NODE_REGISTRY_ID)]
    pub node_registry_program: UncheckedAccount<'info>,
    /// CHECK: node-registry Registry PDA.
    pub registry: UncheckedAccount<'info>,
    /// CHECK: NodeState PDA (optional — mirror is a no-op when absent).
    #[account(mut)]
    pub node: Option<UncheckedAccount<'info>>,

    /// CHECK: SPL token program for the slash transfer.
    pub token_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

impl Dispute<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn dispute(
        &mut self,
        epoch: u64,
        node_id: u64,
        amount: u64,
        severity_bps: u32,
        slash_amount: u64,
        claim_bump: u8,
        authority_bump: u8,
    ) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;

        // Block the payout: this is the very PDA `claim` would have `init`'d.
        self.claim_status.set_inner(ClaimStatus {
            epoch,
            operator: self.operator.key(),
            node_id,
            amount,
            claimed_at: now,
            disputed: true,
            bump: claim_bump,
        });

        let signer: &[&[&[u8]]] = &[&[AUTHORITY_SEED, &[authority_bump]]];
        let node_ai = self.node.as_ref().map(|n| n.to_account_info());

        // Slash the operator's stake (saturates to treasury, mirrors into NodeState).
        if slash_amount > 0 {
            invoke_slash(
                &self.staking_program.to_account_info(),
                &self.program_authority.to_account_info(),
                &self.staking_config.to_account_info(),
                &self.staking_position.to_account_info(),
                &self.staking_vault.to_account_info(),
                &self.staking_treasury.to_account_info(),
                &self.stake_mint.to_account_info(),
                &self.staking_program_authority.to_account_info(),
                &self.node_registry_program.to_account_info(),
                &self.registry.to_account_info(),
                node_ai.as_ref(),
                &self.token_program.to_account_info(),
                signer,
                slash_amount,
            )?;
        }

        // Penalize reputation (multiplicative toward 0.5x, mirrors into NodeState).
        if severity_bps > 0 {
            invoke_penalize(
                &self.reputation_program.to_account_info(),
                &self.program_authority.to_account_info(),
                &self.reputation_config.to_account_info(),
                &self.reputation_state.to_account_info(),
                &self.reputation_program_authority.to_account_info(),
                &self.node_registry_program.to_account_info(),
                &self.registry.to_account_info(),
                node_ai.as_ref(),
                signer,
                severity_bps,
            )?;
        }
        Ok(())
    }
}
