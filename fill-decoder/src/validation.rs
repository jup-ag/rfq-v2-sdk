//! Off-chain validation that mirrors the on-chain `MakerAppearsInOtherInstruction` check.

use crate::transaction::DecodedMessage;

/// Result of checking whether a pubkey is used exclusively in fill instructions.
#[derive(Debug, Clone)]
pub struct ExclusivityReport {
    /// The pubkey that was checked.
    pub pubkey: String,
    /// Instruction indices where this pubkey participates in a fill.
    pub fill_instruction_indices: Vec<usize>,
    /// Instruction indices where this pubkey appears but NO fill was detected.
    pub non_fill_instruction_indices: Vec<usize>,
}

impl ExclusivityReport {
    /// Returns `true` if the pubkey is found in at least one fill instruction
    /// and does **not** appear in any non-fill instruction.
    pub fn is_exclusive(&self) -> bool {
        !self.fill_instruction_indices.is_empty() && self.non_fill_instruction_indices.is_empty()
    }
}

impl std::fmt::Display for ExclusivityReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_exclusive() {
            write!(
                f,
                "✓ {} appears exclusively in fill instruction(s) {:?}",
                self.pubkey, self.fill_instruction_indices,
            )
        } else if self.fill_instruction_indices.is_empty() {
            write!(f, "✗ {} not found in any fill instruction", self.pubkey)
        } else {
            write!(
                f,
                "✗ {} appears in fill instruction(s) {:?} but ALSO in non-fill instruction(s) {:?}",
                self.pubkey, self.fill_instruction_indices, self.non_fill_instruction_indices,
            )
        }
    }
}

/// Check whether a public key (base-58) appears exclusively in instructions
/// that contain a `fill_exact_in`.
///
/// This mirrors the on-chain `MakerAppearsInOtherInstruction` validation:
/// the maker's token accounts must only be referenced by the fill instruction,
/// not by other route legs or programs in the transaction.
///
/// **Note:** For CPI fills (e.g. via Jupiter), the entire Jupiter instruction
/// is treated as "the fill instruction" because account resolution happens at
/// the top-level compiled instruction boundary.
pub fn check_fill_exclusivity(message: &DecodedMessage, pubkey: &str) -> ExclusivityReport {
    let mut fill_indices = Vec::new();
    let mut non_fill_indices = Vec::new();

    for ix in &message.instructions {
        let referenced = ix.accounts.iter().any(|a| a.pubkey == pubkey);
        if !referenced {
            continue;
        }
        if ix.fill.is_some() {
            fill_indices.push(ix.instruction_index);
        } else {
            non_fill_indices.push(ix.instruction_index);
        }
    }

    ExclusivityReport {
        pubkey: pubkey.to_string(),
        fill_instruction_indices: fill_indices,
        non_fill_instruction_indices: non_fill_indices,
    }
}

/// Check multiple public keys at once and return a report for each.
///
/// Convenience wrapper around [`check_fill_exclusivity`] for checking all
/// maker-related accounts in one call.
pub fn check_fill_exclusivity_multi(
    message: &DecodedMessage,
    pubkeys: &[&str],
) -> Vec<ExclusivityReport> {
    pubkeys
        .iter()
        .map(|pk| check_fill_exclusivity(message, pk))
        .collect()
}
