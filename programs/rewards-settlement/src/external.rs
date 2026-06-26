use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::invoke_signed,
    },
};

use crate::error::SettlementError;

pub const GOVERNANCE_ID: Pubkey = pubkey!("8uywvvcGdANC1WM7g1iuEq3crjBwhy5uP5UReKb3xNUE");
pub const STAKING_ID: Pubkey = pubkey!("CvvsFn1SFV2R19WhXmqYyMjbKUWADcej93F6DadHv3yU");
pub const REPUTATION_ID: Pubkey = pubkey!("6E9RfpzBbyshMBrbnSHne8wzjMqQgYPFMDdP2Xra8SEZ");
pub const NODE_REGISTRY_ID: Pubkey = pubkey!("GxhrTKKPybHZPv2MsaLovzKaq9Pq8jHmjNyRMrKZY6aH");
pub const PROTOCOL_CONFIG_SEED: &[u8] = b"protocol_config";

const PROTOCOL_CONFIG_DISCRIMINATOR: [u8; 8] = [0xcf, 0x5b, 0xfa, 0x1c, 0x98, 0xb3, 0xd7, 0xd1];
const PROTOCOL_CONFIG_SPLIT_NODES_OFFSET: usize = 40;
const PROTOCOL_CONFIG_SPLIT_BURN_OFFSET: usize = 44;
const SLASH_DISCRIMINATOR: [u8; 8] = [0xcc, 0x8d, 0x12, 0xa1, 0x08, 0xb1, 0x5c, 0x8e];
const PENALIZE_DISCRIMINATOR: [u8; 8] = [0xc7, 0x6c, 0x20, 0x00, 0xab, 0x4d, 0x91, 0xdf];

pub struct ProtocolSplit {
    pub nodes_bps: u32,
    pub burn_bps: u32,
}

pub fn load_protocol_split(protocol_config: &AccountInfo) -> Result<ProtocolSplit> {
    require_keys_eq!(
        *protocol_config.owner,
        GOVERNANCE_ID,
        SettlementError::Unauthorized
    );
    let data = protocol_config.try_borrow_data()?;
    require!(
        data.len() >= PROTOCOL_CONFIG_SPLIT_BURN_OFFSET + 4,
        SettlementError::Unauthorized
    );
    require!(
        data[..8] == PROTOCOL_CONFIG_DISCRIMINATOR,
        SettlementError::Unauthorized
    );
    let mut nodes = [0u8; 4];
    nodes.copy_from_slice(
        &data[PROTOCOL_CONFIG_SPLIT_NODES_OFFSET..PROTOCOL_CONFIG_SPLIT_NODES_OFFSET + 4],
    );
    let mut burn = [0u8; 4];
    burn.copy_from_slice(
        &data[PROTOCOL_CONFIG_SPLIT_BURN_OFFSET..PROTOCOL_CONFIG_SPLIT_BURN_OFFSET + 4],
    );
    Ok(ProtocolSplit {
        nodes_bps: u32::from_le_bytes(nodes),
        burn_bps: u32::from_le_bytes(burn),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn invoke_slash<'info>(
    staking_program: &AccountInfo<'info>,
    slash_authority: &AccountInfo<'info>,
    config: &AccountInfo<'info>,
    position: &AccountInfo<'info>,
    vault: &AccountInfo<'info>,
    treasury: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    program_authority: &AccountInfo<'info>,
    node_registry_program: &AccountInfo<'info>,
    registry: &AccountInfo<'info>,
    node: Option<&AccountInfo<'info>>,
    token_program: &AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
    amount: u64,
) -> Result<()> {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&SLASH_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    let node_meta = node.map_or(
        AccountMeta::new_readonly(*staking_program.key, false),
        |n| AccountMeta::new(*n.key, false),
    );
    let node_account = node.unwrap_or(staking_program);
    let ix = Instruction {
        program_id: *staking_program.key,
        accounts: vec![
            AccountMeta::new_readonly(*slash_authority.key, true),
            AccountMeta::new_readonly(*config.key, false),
            AccountMeta::new(*position.key, false),
            AccountMeta::new(*vault.key, false),
            AccountMeta::new(*treasury.key, false),
            AccountMeta::new_readonly(*mint.key, false),
            AccountMeta::new_readonly(*program_authority.key, false),
            AccountMeta::new_readonly(*node_registry_program.key, false),
            AccountMeta::new_readonly(*registry.key, false),
            node_meta,
            AccountMeta::new_readonly(*token_program.key, false),
        ],
        data,
    };
    invoke_signed(
        &ix,
        &[
            slash_authority.clone(),
            config.clone(),
            position.clone(),
            vault.clone(),
            treasury.clone(),
            mint.clone(),
            program_authority.clone(),
            node_registry_program.clone(),
            registry.clone(),
            node_account.clone(),
            token_program.clone(),
            staking_program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn invoke_penalize<'info>(
    reputation_program: &AccountInfo<'info>,
    oracle: &AccountInfo<'info>,
    config: &AccountInfo<'info>,
    state: &AccountInfo<'info>,
    program_authority: &AccountInfo<'info>,
    node_registry_program: &AccountInfo<'info>,
    registry: &AccountInfo<'info>,
    node: Option<&AccountInfo<'info>>,
    signer_seeds: &[&[&[u8]]],
    severity_bps: u32,
) -> Result<()> {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&PENALIZE_DISCRIMINATOR);
    data.extend_from_slice(&severity_bps.to_le_bytes());
    let node_meta = node.map_or(
        AccountMeta::new_readonly(*reputation_program.key, false),
        |n| AccountMeta::new(*n.key, false),
    );
    let node_account = node.unwrap_or(reputation_program);
    let ix = Instruction {
        program_id: *reputation_program.key,
        accounts: vec![
            AccountMeta::new_readonly(*oracle.key, true),
            AccountMeta::new_readonly(*config.key, false),
            AccountMeta::new(*state.key, false),
            AccountMeta::new_readonly(*program_authority.key, false),
            AccountMeta::new_readonly(*node_registry_program.key, false),
            AccountMeta::new_readonly(*registry.key, false),
            node_meta,
        ],
        data,
    };
    invoke_signed(
        &ix,
        &[
            oracle.clone(),
            config.clone(),
            state.clone(),
            program_authority.clone(),
            node_registry_program.clone(),
            registry.clone(),
            node_account.clone(),
            reputation_program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}
