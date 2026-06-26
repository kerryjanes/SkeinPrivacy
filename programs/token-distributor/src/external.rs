use anchor_lang::{
    prelude::*,
    solana_program::{
        instruction::{AccountMeta, Instruction},
        program::invoke,
    },
};

pub const WEFT_VESTING_ID: Pubkey = pubkey!("Aa8aMMVmxcA5CKQAJQ3N3EmFtKYTx79hEEYjFYzBCDjb");
const CREATE_SCHEDULE_DISCRIMINATOR: [u8; 8] = [0xc8, 0xb0, 0xd5, 0xd6, 0xd2, 0x79, 0x23, 0xe1];

#[allow(clippy::too_many_arguments)]
pub fn invoke_create_schedule<'info>(
    vesting_program: &AccountInfo<'info>,
    funder: &AccountInfo<'info>,
    beneficiary: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    schedule: &AccountInfo<'info>,
    vault: &AccountInfo<'info>,
    funder_token_account: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    schedule_id: u64,
    total_amount: u64,
    cliff_unlock_amount: u64,
    start_ts: i64,
    cliff_duration: i64,
    duration: i64,
    revocable: bool,
) -> Result<()> {
    let mut data = Vec::with_capacity(57);
    data.extend_from_slice(&CREATE_SCHEDULE_DISCRIMINATOR);
    data.extend_from_slice(&schedule_id.to_le_bytes());
    data.extend_from_slice(&total_amount.to_le_bytes());
    data.extend_from_slice(&cliff_unlock_amount.to_le_bytes());
    data.extend_from_slice(&start_ts.to_le_bytes());
    data.extend_from_slice(&cliff_duration.to_le_bytes());
    data.extend_from_slice(&duration.to_le_bytes());
    data.push(u8::from(revocable));

    let ix = Instruction {
        program_id: *vesting_program.key,
        accounts: vec![
            AccountMeta::new(*funder.key, true),
            AccountMeta::new_readonly(*beneficiary.key, false),
            AccountMeta::new_readonly(*authority.key, false),
            AccountMeta::new_readonly(*mint.key, false),
            AccountMeta::new(*schedule.key, false),
            AccountMeta::new(*vault.key, false),
            AccountMeta::new(*funder_token_account.key, false),
            AccountMeta::new_readonly(*token_program.key, false),
            AccountMeta::new_readonly(*system_program.key, false),
        ],
        data,
    };
    invoke(
        &ix,
        &[
            funder.clone(),
            beneficiary.clone(),
            authority.clone(),
            mint.clone(),
            schedule.clone(),
            vault.clone(),
            funder_token_account.clone(),
            token_program.clone(),
            system_program.clone(),
            vesting_program.clone(),
        ],
    )?;
    Ok(())
}
