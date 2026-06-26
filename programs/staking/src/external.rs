use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::invoke_signed,
    },
};

pub const NODE_REGISTRY_ID: Pubkey = pubkey!("GxhrTKKPybHZPv2MsaLovzKaq9Pq8jHmjNyRMrKZY6aH");
pub const SET_STAKE_DISCRIMINATOR: [u8; 8] = [0x56, 0xad, 0xe9, 0xc4, 0x78, 0x85, 0x67, 0xfe];

pub fn invoke_set_stake<'info>(
    node_registry_program: &AccountInfo<'info>,
    staking_authority: &AccountInfo<'info>,
    registry: &AccountInfo<'info>,
    node: &AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
    amount: u64,
) -> Result<()> {
    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&SET_STAKE_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    let ix = Instruction {
        program_id: *node_registry_program.key,
        accounts: vec![
            AccountMeta::new_readonly(*staking_authority.key, true),
            AccountMeta::new_readonly(*registry.key, false),
            AccountMeta::new(*node.key, false),
        ],
        data,
    };
    invoke_signed(
        &ix,
        &[
            staking_authority.clone(),
            registry.clone(),
            node.clone(),
            node_registry_program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}
