//! Constants, detection, and low-level decoding for `fill_exact_in` instructions.

use crate::error::FillDecoderError;
use crate::types::{FillAccounts, FillExactInInstruction};

/// Base-58 encoded RFQ v2 program ID.
pub const RFQ_V2_PROGRAM_ID: &str = "fd3nMFYTQjX1yr5ER8u7tPdHJB7qt8RpDpNtLQX2Br5";

/// 8-byte Anchor discriminator for `fill_exact_in`.
pub const FILL_EXACT_IN_DISCRIMINATOR: [u8; 8] = [222, 208, 6, 209, 154, 163, 54, 94];

/// Number of accounts the `fill_exact_in` instruction expects.
pub const FILL_EXACT_IN_ACCOUNT_COUNT: usize = 11;

/// Labels for the 11 accounts in a `fill_exact_in` instruction (from the IDL).
pub const FILL_ACCOUNT_LABELS: [&str; FILL_EXACT_IN_ACCOUNT_COUNT] = [
    "user",
    "fill_authority",
    "user_base_token_account",
    "user_quote_token_account",
    "maker_base_token_account",
    "maker_quote_token_account",
    "base_mint",
    "quote_mint",
    "base_token_program",
    "quote_token_program",
    "instructions_sysvar",
];

/// Returns `true` if `instruction_data` begins with the `fill_exact_in` discriminator.
pub fn is_fill_exact_in(instruction_data: &[u8]) -> bool {
    instruction_data
        .get(..8)
        .is_some_and(|d| d == FILL_EXACT_IN_DISCRIMINATOR)
}

pub(crate) fn read_u8(data: &[u8], offset: &mut usize) -> crate::Result<u8> {
    if *offset + 1 > data.len() {
        return Err(FillDecoderError::other(
            "unexpected end of instruction data",
        ));
    }
    let val = data[*offset];
    *offset += 1;
    Ok(val)
}

/// Decode the `fill_exact_in` instruction from raw instruction data bytes.
pub fn decode_fill_instruction(data: &[u8]) -> crate::Result<FillExactInInstruction> {
    if !is_fill_exact_in(data) {
        return Err(FillDecoderError::validation(
            "instruction data does not match fill_exact_in discriminator",
        ));
    }
    borsh::from_slice::<FillExactInInstruction>(&data[8..])
        .map_err(|e| FillDecoderError::other(format!("borsh deserialization failed: {e}")))
}

/// Map the instruction's account keys (in order) to named [`FillAccounts`].
///
/// `account_keys` must contain at least [`FILL_EXACT_IN_ACCOUNT_COUNT`] entries.
pub fn decode_fill_accounts(account_keys: &[[u8; 32]]) -> crate::Result<FillAccounts> {
    if account_keys.len() < FILL_EXACT_IN_ACCOUNT_COUNT {
        return Err(FillDecoderError::validation(format!(
            "expected at least {} account keys, got {}",
            FILL_EXACT_IN_ACCOUNT_COUNT,
            account_keys.len()
        )));
    }

    // The length check above guarantees indices 0..=10.
    Ok(FillAccounts {
        user: account_keys[0],
        fill_authority: account_keys[1],
        user_base_token_account: account_keys[2],
        user_quote_token_account: account_keys[3],
        maker_base_token_account: account_keys[4],
        maker_quote_token_account: account_keys[5],
        base_mint: account_keys[6],
        quote_mint: account_keys[7],
        base_token_program: account_keys[8],
        quote_token_program: account_keys[9],
        instructions_sysvar: account_keys[10],
    })
}
