//! Reproduces a "build-the-aggregator-tx-yourself" attack against an RFQ v2
//! market maker, then proves the maker is protected.
//!
//! Scenario
//! --------
//! A malicious *taker* hand-builds a Jupiter aggregator `route_v2` transaction
//! that routes through a specific market maker (`fill_authority`) and fills in
//! whatever amounts they like for the token-out side. We construct two variants
//! against the testing maker `917Yp1mesMs14d32kDwH4uNocdhuB67QzzaYKezkjy4B`:
//!
//!   * `honest`    – a single `route_v2` instruction whose `route_plan` contains
//!                   one `JupiterRfqV2` step (tag 120) wrapping a `fill_exact_in`.
//!                   The maker's accounts appear *only* inside the fill.
//!   * `malicious` – the same route, plus an extra SPL-token `transfer` that
//!                   tries to move the maker's base-token balance out using the
//!                   maker's own `fill_authority` as the transfer authority.
//!                   This is the user "building the tx as they wish".
//!
//! Both are emitted as base64 v0 `VersionedTransaction`s (placeholder
//! signatures — a real submission needs the maker's signature, which is the
//! whole point) and then decoded with `fill-decoder`.
//!
//! Why the maker is safe
//! ---------------------
//! Two on-chain guards in `rfq_v2.json` cover this:
//!   1. `fill_authority` is a **required signer** of `fill_exact_in`. The taker
//!      can write any `levels` / out-amount they want, but without the maker's
//!      signature over *those exact bytes* the tx never lands.
//!   2. `MakerAppearsInOtherInstruction` (error 6005): the program reads the
//!      instructions sysvar and rejects the fill if the maker authority shows
//!      up in any *other* instruction — exactly the malicious variant.
//!
//! `fill_decoder::check_fill_exclusivity` is the off-chain mirror of guard #2.
//! Running this example asserts the honest tx is exclusive and the malicious
//! one is not. To confirm the *on-chain* fix, submit the printed malicious
//! base64 to preprod and expect `MakerAppearsInOtherInstruction`.
//!
//! Run with:
//!   cargo run -p market-maker-client-sdk --example maker_safety

use std::str::FromStr;

use base64::prelude::*;
use fill_decoder::{check_fill_exclusivity, decode_transaction_base64, RFQ_V2_PROGRAM_ID};
use solana_sdk::{
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    message::{v0, VersionedMessage},
    pubkey::Pubkey,
    signature::Signature,
    transaction::VersionedTransaction,
};

// ---------------------------------------------------------------------------
// Fixed program / account addresses (from the IDLs and the e2e test fixtures).
// ---------------------------------------------------------------------------

const JUPITER_PROGRAM_ID: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
const JUPITER_EVENT_AUTHORITY: &str = "D8cy77BBepLMngZx6ZukaTff5hCt1HrWyKk3Hnd9oitf";
const INSTRUCTIONS_SYSVAR: &str = "Sysvar1nstructions1111111111111111111111111";
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

/// The testing market maker we're probing — this is the RFQ `fill_authority`.
const MAKER_FILL_AUTHORITY: &str = "917Yp1mesMs14d32kDwH4uNocdhuB67QzzaYKezkjy4B";
/// The maker's token accounts (from the real preprod fill in the e2e suite).
const MAKER_BASE_TOKEN_ACCOUNT: &str = "FmQGEXvc2houbBgw1HVPYf7gA6JBxzhCMUQWK1tky7B9";
const MAKER_QUOTE_TOKEN_ACCOUNT: &str = "FUU2uSdMnTVcZWesD5Fen8AJUs7mSMdnM6qKMUCnqVw6";

/// A3QA (base) and USDC (quote) — the pair this maker quotes on preprod.
const BASE_MINT: &str = "A3QAoKnf3jFcCfTGvEpE7KVBMZqXQJwvwt6Uc4UExkDp";
const QUOTE_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

/// The (attacker) taker.
const TAKER: &str = "B8ttfFCJRyJivDLn19Q6uvndVCssTwkokLAgz22vyo1Q";

// route_v2 discriminator (jupiter_aggregator.json).
const ROUTE_V2_DISCRIMINATOR: [u8; 8] = [187, 100, 250, 204, 49, 196, 175, 20];
// fill_exact_in discriminator (rfq_v2.json).
const FILL_EXACT_IN_DISCRIMINATOR: [u8; 8] = [222, 208, 6, 209, 154, 163, 54, 94];
// Swap::JupiterRfqV2 enum tag in the aggregator's `Swap` enum.
const SWAP_TAG_JUPITER_RFQ_V2: u8 = 120;

/// Taker side, matching both IDLs' `Side` enum (Bid = 0, Ask = 1).
#[derive(Clone, Copy)]
#[allow(dead_code)] // Ask is part of the model; this scenario exercises Bid.
enum Side {
    Bid = 0,
    Ask = 1,
}

/// One orderbook level — `Level` in rfq_v2.json.
struct Level {
    px_ticks: u64,
    qty_lots: u64,
}

/// The taker-chosen fill parameters. In a real flow these come from the maker's
/// *signed* quote; here the attacker writes them freely to dramatize the point.
struct FillSpec {
    side: Side,
    amount_in_atoms: u64,
    expire_at: u64,
    tick_size_qpb: u64,
    lot_size_base: u64,
    levels: Vec<Level>,
}

fn pk(s: &str) -> Pubkey {
    Pubkey::from_str(s).expect("valid base58 pubkey")
}

/// Two deterministic stand-in token accounts for the taker (no ATA derivation
/// needed for a decode-only fixture).
fn taker_base_token_account() -> Pubkey {
    Pubkey::new_from_array([0x11; 32])
}
fn taker_quote_token_account() -> Pubkey {
    Pubkey::new_from_array([0x22; 32])
}

/// Borsh-encode the `fill_exact_in` payload that Jupiter carries as the
/// `fill_data: bytes` of a `JupiterRfqV2` route step:
///
///   FILL_DISCRIMINATOR(8) | taker_side(1) | amount_in_atoms(8) | params
///   params = expire_at(8) | tick_size_qpb(8) | lot_size_base(8)
///            | levels: u32 len + N * (px_ticks u64, qty_lots u64)
fn encode_fill_data(spec: &FillSpec) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&FILL_EXACT_IN_DISCRIMINATOR);
    d.push(spec.side as u8);
    d.extend_from_slice(&spec.amount_in_atoms.to_le_bytes());
    d.extend_from_slice(&spec.expire_at.to_le_bytes());
    d.extend_from_slice(&spec.tick_size_qpb.to_le_bytes());
    d.extend_from_slice(&spec.lot_size_base.to_le_bytes());
    d.extend_from_slice(&(spec.levels.len() as u32).to_le_bytes());
    for lvl in &spec.levels {
        d.extend_from_slice(&lvl.px_ticks.to_le_bytes());
        d.extend_from_slice(&lvl.qty_lots.to_le_bytes());
    }
    d
}

/// Borsh-encode `route_v2` instruction data with a single `JupiterRfqV2` step.
///
///   disc(8) | in_amount u64 | quoted_out_amount u64 | slippage_bps u16
///   | platform_fee_bps u16 | positive_slippage_bps u16
///   | route_plan: u32 len + steps
///   step = swap | bps u16 | input_index u8 | output_index u8
///   swap(JupiterRfqV2) = tag(120) | side u8 | fill_data: u32 len + bytes
fn encode_route_v2_data(route_in_amount: u64, quoted_out_amount: u64, spec: &FillSpec) -> Vec<u8> {
    let fill_data = encode_fill_data(spec);

    let mut d = Vec::new();
    d.extend_from_slice(&ROUTE_V2_DISCRIMINATOR);
    d.extend_from_slice(&route_in_amount.to_le_bytes());
    d.extend_from_slice(&quoted_out_amount.to_le_bytes());
    d.extend_from_slice(&0u16.to_le_bytes()); // slippage_bps
    d.extend_from_slice(&0u16.to_le_bytes()); // platform_fee_bps
    d.extend_from_slice(&0u16.to_le_bytes()); // positive_slippage_bps

    d.extend_from_slice(&1u32.to_le_bytes()); // route_plan length = 1
                                              // --- step 0: JupiterRfqV2 ---
    d.push(SWAP_TAG_JUPITER_RFQ_V2);
    d.push(spec.side as u8);
    d.extend_from_slice(&(fill_data.len() as u32).to_le_bytes());
    d.extend_from_slice(&fill_data);
    d.extend_from_slice(&10_000u16.to_le_bytes()); // bps = 100% of the route
    d.push(0u8); // input_index  (0 => route in_amount flows straight into this leg)
    d.push(1u8); // output_index

    d
}

/// Build the `route_v2` instruction. The fixed `route_v2` accounts come first
/// (per jupiter_aggregator.json), then Jupiter appends the per-step CPI
/// accounts as remaining_accounts: the RFQ program id followed by the 11
/// `fill_exact_in` accounts in IDL order.
fn build_route_v2_instruction(spec: &FillSpec, quoted_out_amount: u64) -> Instruction {
    let taker = pk(TAKER);
    let maker = pk(MAKER_FILL_AUTHORITY);

    // 11 fill_exact_in accounts (rfq_v2.json order). `user` is a writable
    // signer and `fill_authority` (the maker) is a signer — the decoder relies
    // on those flags to locate the embedded fill block.
    let fill_block = vec![
        AccountMeta::new(taker, true),                          // 0 user
        AccountMeta::new_readonly(maker, true),                 // 1 fill_authority
        AccountMeta::new(taker_base_token_account(), false),    // 2 user_base_token_account
        AccountMeta::new(taker_quote_token_account(), false),   // 3 user_quote_token_account
        AccountMeta::new(pk(MAKER_BASE_TOKEN_ACCOUNT), false),  // 4 maker_base_token_account
        AccountMeta::new(pk(MAKER_QUOTE_TOKEN_ACCOUNT), false), // 5 maker_quote_token_account
        AccountMeta::new_readonly(pk(BASE_MINT), false),        // 6 base_mint
        AccountMeta::new_readonly(pk(QUOTE_MINT), false),       // 7 quote_mint
        AccountMeta::new_readonly(pk(TOKEN_PROGRAM), false),    // 8 base_token_program
        AccountMeta::new_readonly(pk(TOKEN_PROGRAM), false),    // 9 quote_token_program
        AccountMeta::new_readonly(pk(INSTRUCTIONS_SYSVAR), false), // 10 instructions_sysvar
    ];

    // route_v2 fixed accounts (no optional destination_token_account).
    let mut accounts = vec![
        AccountMeta::new(taker, true),                        // user_transfer_authority
        AccountMeta::new(taker_quote_token_account(), false), // user_source_token_account
        AccountMeta::new(taker_base_token_account(), false),  // user_destination_token_account
        AccountMeta::new_readonly(pk(QUOTE_MINT), false),     // source_mint
        AccountMeta::new_readonly(pk(BASE_MINT), false),      // destination_mint
        AccountMeta::new_readonly(pk(TOKEN_PROGRAM), false),  // source_token_program
        AccountMeta::new_readonly(pk(TOKEN_PROGRAM), false),  // destination_token_program
        AccountMeta::new_readonly(pk(JUPITER_EVENT_AUTHORITY), false), // event_authority
        AccountMeta::new_readonly(pk(JUPITER_PROGRAM_ID), false), // program
        // remaining_accounts: RFQ program id marks the start of the CPI block.
        AccountMeta::new_readonly(pk(RFQ_V2_PROGRAM_ID), false),
    ];
    accounts.extend(fill_block);

    Instruction {
        program_id: pk(JUPITER_PROGRAM_ID),
        accounts,
        data: encode_route_v2_data(spec.amount_in_atoms, quoted_out_amount, spec),
    }
}

/// A bare SPL-token `transfer` (instruction tag 3) that tries to move the
/// maker's base-token balance to the taker, authorized by the maker's own
/// `fill_authority`. This is the extra instruction a malicious taker would
/// staple on — and exactly what guard 6005 forbids.
fn build_maker_drain_instruction(amount: u64) -> Instruction {
    let mut data = vec![3u8]; // SPL Token: Transfer
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id: pk(TOKEN_PROGRAM),
        accounts: vec![
            AccountMeta::new(pk(MAKER_BASE_TOKEN_ACCOUNT), false), // source (maker funds)
            AccountMeta::new(taker_base_token_account(), false),   // destination (taker)
            AccountMeta::new_readonly(pk(MAKER_FILL_AUTHORITY), true), // authority = maker
        ],
        data,
    }
}

/// Compile instructions into a base64 v0 VersionedTransaction with placeholder
/// signatures (the maker's real signature is intentionally absent).
fn to_base64_tx(instructions: &[Instruction]) -> String {
    let payer = pk(TAKER);
    let message = v0::Message::try_compile(&payer, instructions, &[], Hash::default())
        .expect("failed to compile v0 message");
    let num_sigs = message.header.num_required_signatures as usize;
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); num_sigs],
        message: VersionedMessage::V0(message),
    };
    BASE64_STANDARD.encode(bincode::serialize(&tx).expect("serialize tx"))
}

/// Decode + report the embedded fill and the maker-exclusivity verdict.
/// Returns `true` if every maker account is used exclusively inside the fill.
fn analyze(label: &str, tx_b64: &str) -> bool {
    println!("\n=================== {label} ===================");
    println!("base64 transaction:\n{tx_b64}\n");

    let tx = decode_transaction_base64(tx_b64).expect("decode failed");

    // Show the fill the taker tried to push through.
    for ix in &tx.message.instructions {
        if let Some((fill, analysis)) = &ix.fill {
            println!("Embedded RFQ v2 fill (instruction #{}):", ix.instruction_index);
            println!("  taker_side       : {}", fill.taker_side);
            println!("  amount_in_atoms  : {}", fill.amount_in_atoms);
            println!("  tick_size_qpb    : {}", fill.params.tick_size_qpb);
            println!("  lot_size_base    : {}", fill.params.lot_size_base);
            for (i, l) in fill.params.levels.iter().enumerate() {
                println!("  level[{i}]         : px_ticks={}, qty_lots={}", l.px_ticks, l.qty_lots);
            }
            println!("  --> amount_spent : {} atoms", analysis.amount_spent_atoms);
            println!("  --> amount_out   : {} atoms (taker-chosen pricing)", analysis.amount_out_atoms);
        }
    }

    // The off-chain mirror of on-chain guard 6005.
    let maker_keys = [
        MAKER_FILL_AUTHORITY,
        MAKER_BASE_TOKEN_ACCOUNT,
        MAKER_QUOTE_TOKEN_ACCOUNT,
    ];
    println!("\nMaker-exclusivity (mirror of MakerAppearsInOtherInstruction / 6005):");
    let mut all_exclusive = true;
    for key in maker_keys {
        let report = check_fill_exclusivity(&tx.message, key);
        println!("  {report}");
        all_exclusive &= report.is_exclusive();
    }
    all_exclusive
}

fn main() {
    println!("Probing market maker {MAKER_FILL_AUTHORITY}");
    println!("(reproducing a hand-built aggregator tx with attacker-chosen out amounts)");

    // The taker writes an absurdly favorable book: pay ~1000 quote atoms,
    // walk away with 1,000,000,000 base atoms. The maker would never sign this.
    let spec = FillSpec {
        side: Side::Bid, // taker buys base (A3QA) with quote (USDC)
        amount_in_atoms: 1_000_000,
        expire_at: 9_999_999_999,
        tick_size_qpb: 1,
        lot_size_base: 1_000_000,
        levels: vec![Level {
            px_ticks: 1,
            qty_lots: 1_000,
        }],
    };
    // quoted_out_amount is another field the taker controls freely.
    let quoted_out_amount = u64::MAX;

    // ---- honest: maker accounts live only inside the fill ----
    let honest = to_base64_tx(&[build_route_v2_instruction(&spec, quoted_out_amount)]);
    let honest_ok = analyze("HONEST (single route_v2 leg)", &honest);

    // ---- malicious: also drain the maker via a second instruction ----
    let malicious = to_base64_tx(&[
        build_route_v2_instruction(&spec, quoted_out_amount),
        build_maker_drain_instruction(1_000_000_000),
    ]);
    let malicious_ok = analyze("MALICIOUS (route_v2 + maker-drain transfer)", &malicious);

    println!("\n========================= VERDICT =========================");
    println!("honest    : maker accounts exclusive to fill = {honest_ok}  (would pass on-chain)");
    println!("malicious : maker accounts exclusive to fill = {malicious_ok}  (rejected by 6005)");
    println!(
        "\nMM funds are safe: the taker can hand-build any route and any out amount,\n\
         but (1) the maker must SIGN the fill, and (2) the maker authority/accounts\n\
         cannot appear in any other instruction. Submit the MALICIOUS base64 to\n\
         preprod to confirm the on-chain `MakerAppearsInOtherInstruction` rejection."
    );

    // Make the example a usable pass/fail probe for the off-chain mirror.
    assert!(honest_ok, "honest tx should be exclusive — maker is only in the fill");
    assert!(
        !malicious_ok,
        "malicious tx must be flagged — maker appears in a non-fill instruction"
    );
    println!("\nOK: exclusivity guard behaves as expected.");
}
