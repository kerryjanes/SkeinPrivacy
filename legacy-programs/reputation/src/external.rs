use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::invoke_signed,
    },
};

pub const NODE_REGISTRY_ID: Pubkey = pubkey!("GxhrTKKPybHZPv2MsaLovzKaq9Pq8jHmjNyRMrKZY6aH");
pub const SET_REPUTATION_DISCRIMINATOR: [u8; 8] = [0x8c, 0x22, 0xb3, 0x24, 0x43, 0x55, 0x3a, 0xe1];

pub fn invoke_set_reputation<'info>(
    node_registry_program: &AccountInfo<'info>,
    reputation_authority: &AccountInfo<'info>,
    registry: &AccountInfo<'info>,
    node: &AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
    reputation_bps: u16,
) -> Result<()> {
    let mut data = Vec::with_capacity(10);
    data.extend_from_slice(&SET_REPUTATION_DISCRIMINATOR);
    data.extend_from_slice(&reputation_bps.to_le_bytes());
    let ix = Instruction {
        program_id: *node_registry_program.key,
        accounts: vec![
            AccountMeta::new_readonly(*reputation_authority.key, true),
            AccountMeta::new_readonly(*registry.key, false),
            AccountMeta::new(*node.key, false),
        ],
        data,
    };
    invoke_signed(
        &ix,
        &[
            reputation_authority.clone(),
            registry.clone(),
            node.clone(),
            node_registry_program.clone(),
        ],
        signer_seeds,
    )?;
    Ok(())
}
