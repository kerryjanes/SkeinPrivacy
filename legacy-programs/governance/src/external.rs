use anchor_lang::prelude::*;

use crate::error::GovError;

pub const STAKING_ID: Pubkey = pubkey!("CvvsFn1SFV2R19WhXmqYyMjbKUWADcej93F6DadHv3yU");
pub const STAKE_SEED: &[u8] = b"stake";
const STAKE_POSITION_DISCRIMINATOR: [u8; 8] = [0x4e, 0xa5, 0x1e, 0x6f, 0xab, 0x7d, 0x0b, 0xdc];
const STAKE_POSITION_LEN: usize = 145;
const STAKE_POSITION_OPERATOR_OFFSET: usize = 8;
const STAKE_POSITION_NODE_ID_OFFSET: usize = 40;
const STAKE_POSITION_AMOUNT_OFFSET: usize = 112;
const STAKE_POSITION_LOCKED_UNTIL_OFFSET: usize = 128;

#[derive(Clone, Copy)]
pub struct StakePositionView {
    pub amount: u64,
    pub locked_until: i64,
}

pub fn load_stake_position(
    position: &AccountInfo,
    operator: &Pubkey,
    node_id: u64,
) -> Result<StakePositionView> {
    require_keys_eq!(*position.owner, STAKING_ID, GovError::InvalidPositionOwner);
    let expected = Pubkey::find_program_address(
        &[STAKE_SEED, operator.as_ref(), &node_id.to_le_bytes()],
        &STAKING_ID,
    )
    .0;
    require_keys_eq!(*position.key, expected, GovError::PositionOperatorMismatch);

    let data = position.try_borrow_data()?;
    require!(
        data.len() >= STAKE_POSITION_LEN,
        GovError::InvalidPositionOwner
    );
    require!(
        data[..8] == STAKE_POSITION_DISCRIMINATOR,
        GovError::InvalidPositionOwner
    );

    let mut operator_bytes = [0u8; 32];
    operator_bytes.copy_from_slice(
        &data[STAKE_POSITION_OPERATOR_OFFSET..STAKE_POSITION_OPERATOR_OFFSET + 32],
    );
    require_keys_eq!(
        Pubkey::new_from_array(operator_bytes),
        *operator,
        GovError::PositionOperatorMismatch
    );

    let mut node_bytes = [0u8; 8];
    node_bytes
        .copy_from_slice(&data[STAKE_POSITION_NODE_ID_OFFSET..STAKE_POSITION_NODE_ID_OFFSET + 8]);
    require!(
        u64::from_le_bytes(node_bytes) == node_id,
        GovError::PositionOperatorMismatch
    );

    let mut amount_bytes = [0u8; 8];
    amount_bytes
        .copy_from_slice(&data[STAKE_POSITION_AMOUNT_OFFSET..STAKE_POSITION_AMOUNT_OFFSET + 8]);
    let mut locked_bytes = [0u8; 8];
    locked_bytes.copy_from_slice(
        &data[STAKE_POSITION_LOCKED_UNTIL_OFFSET..STAKE_POSITION_LOCKED_UNTIL_OFFSET + 8],
    );
    Ok(StakePositionView {
        amount: u64::from_le_bytes(amount_bytes),
        locked_until: i64::from_le_bytes(locked_bytes),
    })
}
