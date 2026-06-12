//! # fill-decoder
//!
//! Decoder and analysis utilities for RFQ v2 `fill_exact_in` transactions on Solana.

pub mod aggregator;
pub mod analysis;
pub mod decode;
pub mod error;
pub mod lookups;
pub mod scanner;
pub mod transaction;
pub mod types;
pub mod validation;
pub use error::{FillDecoderError, Result};

pub use types::{
    FillAccounts, FillAnalysis, FillExactInInstruction, FillExactInParams, FillMints, Level, Side,
};

pub use decode::{
    decode_fill_accounts, decode_fill_instruction, is_fill_exact_in, FILL_ACCOUNT_LABELS,
    FILL_EXACT_IN_ACCOUNT_COUNT, FILL_EXACT_IN_DISCRIMINATOR, RFQ_V2_PROGRAM_ID,
};

pub use analysis::analyze_fill;

pub use scanner::scan_for_embedded_fill;

pub use aggregator::{
    decode_jupiter_rfq_fill, decode_jupiter_rfq_step_indices, extract_platform_fee_bps,
    route_mint_positions, JupiterRfqStepInfo, AGGREGATOR_IDL_JSON, JUPITER_PROGRAM_ID,
};

/// The Anchor IDL for the RFQ v2 program, embedded at compile time.
pub const IDL_JSON: &str = include_str!("../idls/rfq_v2.json");

pub use transaction::{
    decode_message_base64, decode_transaction_base64, decode_transaction_bytes, AddressTableLookup,
    DecodedInstruction, DecodedMessage, DecodedTransaction, MessageHeader, MessageVersion,
    ResolvedAccount,
};

pub use validation::{
    all_exclusive, check_fill_exclusivity, check_fill_exclusivity_multi, ExclusivityReport,
};

pub use lookups::{parse_lookup_table_addresses, resolve_address_lookups, LookupTableMap};

#[cfg(test)]
mod tests {
    use super::*;

    /// Build minimal fill_exact_in instruction data for testing.
    fn build_instruction_data(
        side: Side,
        amount_in: u64,
        expire_at: u64,
        tick_size_qpb: u64,
        lot_size_base: u64,
        levels: &[(u64, u64)],
    ) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&FILL_EXACT_IN_DISCRIMINATOR);
        data.push(side as u8);
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&expire_at.to_le_bytes());
        data.extend_from_slice(&tick_size_qpb.to_le_bytes());
        data.extend_from_slice(&lot_size_base.to_le_bytes());
        data.extend_from_slice(&(levels.len() as u32).to_le_bytes());
        for (px, qty) in levels {
            data.extend_from_slice(&px.to_le_bytes());
            data.extend_from_slice(&qty.to_le_bytes());
        }
        data
    }

    #[test]
    fn test_discriminator_check() {
        let data = build_instruction_data(Side::Bid, 0, 0, 1, 1, &[]);
        assert!(is_fill_exact_in(&data));
        assert!(!is_fill_exact_in(&[0u8; 8]));
        assert!(!is_fill_exact_in(&[0u8; 4]));
    }

    #[test]
    fn test_decode_roundtrip() {
        let levels = vec![(100, 50), (105, 30)];
        let data = build_instruction_data(Side::Ask, 1_000_000, 999, 1_000, 1_000_000, &levels);

        let ix = decode_fill_instruction(&data).unwrap();
        assert_eq!(ix.taker_side, Side::Ask);
        assert_eq!(ix.amount_in_atoms, 1_000_000);
        assert_eq!(ix.params.expire_at, 999);
        assert_eq!(ix.params.tick_size_qpb, 1_000);
        assert_eq!(ix.params.lot_size_base, 1_000_000);
        assert_eq!(ix.params.levels.len(), 2);
        assert_eq!(ix.params.levels[0].px_ticks, 100);
        assert_eq!(ix.params.levels[0].qty_lots, 50);
        assert_eq!(ix.params.levels[1].px_ticks, 105);
        assert_eq!(ix.params.levels[1].qty_lots, 30);
    }

    #[test]
    fn test_decode_accounts() {
        let keys: Vec<[u8; 32]> = (0..11).map(|i| [i as u8; 32]).collect();
        let accs = decode_fill_accounts(&keys).unwrap();
        assert_eq!(accs.user, [0u8; 32]);
        assert_eq!(accs.fill_authority, [1u8; 32]);
        assert_eq!(accs.maker_base_token_account, [4u8; 32]);
        assert_eq!(accs.quote_mint, [7u8; 32]);
    }

    #[test]
    fn test_decode_accounts_too_few() {
        let keys: Vec<[u8; 32]> = (0..5).map(|i| [i as u8; 32]).collect();
        assert!(decode_fill_accounts(&keys).is_err());
    }

    // SOL/USDC example: lot_size_base = 1 (raw), tick_size_qpb = 1
    // Taker buys SOL with 500 USDC atoms worth, best ask px_ticks = 100
    // price_per_lot = 100 * 1 = 100 quote-atoms per lot
    // affordable lots = 500 / 100 = 5
    // base out = 5 * 1 = 5 base-atoms
    #[test]
    fn test_analyze_bid_single_level() {
        let data = build_instruction_data(
            Side::Bid,
            500,  // 500 quote-atoms in
            9999, // expire_at
            1,    // tick_size_qpb
            1,    // lot_size_base (raw)
            &[(100, 10)],
        );
        let ix = decode_fill_instruction(&data).unwrap();
        let analysis = analyze_fill(&ix).unwrap();

        assert_eq!(analysis.taker_side, Side::Bid);
        assert_eq!(analysis.amount_spent_atoms, 500);
        assert_eq!(analysis.amount_out_atoms, 5);
        assert_eq!(analysis.total_lots_filled, 5);
        assert_eq!(analysis.vwap_ticks, 100);
        assert_eq!(analysis.levels_consumed, 1);
    }

    // Multi-level bid: 2 ask levels at different prices
    #[test]
    fn test_analyze_bid_multi_level() {
        // 1000 quote-atoms, two levels:
        //   level 0: px=100, qty=5  → spend 500, get 5 lots
        //   level 1: px=200, qty=5  → spend 400 (afford 2), get 2 lots
        // total spent = 900, remaining = 100, out = 7 lots = 7 atoms
        let data = build_instruction_data(
            Side::Bid,
            1000,
            9999,
            1, // tick_size_qpb
            1, // lot_size_base
            &[(100, 5), (200, 5)],
        );
        let ix = decode_fill_instruction(&data).unwrap();
        let analysis = analyze_fill(&ix).unwrap();

        assert_eq!(analysis.amount_spent_atoms, 900);
        assert_eq!(analysis.amount_out_atoms, 7);
        assert_eq!(analysis.total_lots_filled, 7);
        assert_eq!(analysis.levels_consumed, 2);
        // VWAP = (100*5 + 200*2) / 7 = 900/7 = 128 (integer division)
        assert_eq!(analysis.vwap_ticks, 128);
    }

    // Ask side: taker sells base, receives quote
    #[test]
    fn test_analyze_ask_single_level() {
        // Taker sells 10 base-atoms, lot_size = 2, so 5 lots available
        // Bid level: px=50, qty=10 → take 5 lots
        // quote out = 5 * (50 * 1) = 250
        let data = build_instruction_data(
            Side::Ask,
            10,   // 10 base-atoms in
            9999, // expire_at
            1,    // tick_size_qpb
            2,    // lot_size_base
            &[(50, 10)],
        );
        let ix = decode_fill_instruction(&data).unwrap();
        let analysis = analyze_fill(&ix).unwrap();

        assert_eq!(analysis.taker_side, Side::Ask);
        assert_eq!(analysis.amount_spent_atoms, 10);
        assert_eq!(analysis.amount_out_atoms, 250);
        assert_eq!(analysis.total_lots_filled, 5);
        assert_eq!(analysis.vwap_ticks, 50);
    }

    #[test]
    fn test_effective_price_bid() {
        let data = build_instruction_data(Side::Bid, 500, 9999, 1, 1, &[(100, 10)]);
        let ix = decode_fill_instruction(&data).unwrap();
        let analysis = analyze_fill(&ix).unwrap();

        // Paid 500 quote-atoms for 5 base-atoms → price = 100.0
        assert!((analysis.effective_price() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_display() {
        let data = build_instruction_data(Side::Bid, 500, 9999, 1, 1, &[(100, 10)]);
        let ix = decode_fill_instruction(&data).unwrap();
        let analysis = analyze_fill(&ix).unwrap();
        let s = format!("{}", analysis);
        assert!(s.contains("Bid"));
        assert!(s.contains("vwap_ticks: 100"));
    }

    // ---- Transaction / Message decoding tests ----

    /// Real transaction from Solana mainnet containing a fill_exact_in instruction.
    const REAL_TX_BASE64: &str = "AsPqw9SAB7rMKDuWgFVxTnfagAj/mSIwuKrYVM3csciSD2HOcJfht8nYL9sARghcVsJlxtTT0uaudrmCDEV1PwkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAIBBg7KTUteDZUF9eqHCzJvdWHUARq8NQU4DIdyCSngydvcb3bk5le5izsp4223iXZnE4grlusUmzXQL4KHUihpkIa4ZNs5dmgwxMyL/IfP1O5Ac9iAQLnbqGpcsdM0PwusuV3sGjl7eLbP6JfIktH4I1aEAyciR2HwKu4QXwEGaOhOz2Hiyz7h80+tj7g8An6p3AGnu96N6DCinehLp7TnorlL9sMrLW9QqpZr3Vb5mV0mnsAM1mxk/2i/SD2e+0t5s5TXDN1sPYOMmaxV5QajzizK3Ud8JdMKkPML/GYipWTOIZRVvlKNhT2MVZBGmyT2HRRgChORTrPsbIfA4a0nvrlLAwZGb+UhFzL/7K26csOb57yM5bvF9xJrLEObOkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAR51VvyMcBu7nTFbs5oFQf9sbLeo/SOUQKxzaJWvBOPBt324ddloZPZy+FGzut5rBy0he1fWzeROoz1hX7/AKmMlyWPTiSJ8bs9ECkUjg2DC1oTmdr/EIQEjnvY2+n4WQnk1I8BnjipjWjfEIVY5ELgMj6qznalm0dryLHTf7MuPFxhbWYWCgjj9eAH/Sz/1aYw9GQui88WwYxH3hd9dvYHCAAFAtmOAwAIAAkDzPEfAAAAAAAJAgACDAIAAADwSlABAAAAAAoFAgAZCwkJk/F7ZPSErnb/DAYAAwAdCQsBAQo0AAIDGR0LCwoaCg4NAAECBAUGGRsLCyAhACIWBwQXGBcYGAsgCh4AHw8HAxAREgsTFBUKHGO7ZPrMMcSvFAAtMQEAAAAAYHrfdQAAAABkAAoAAAADAAAAeAEsAAAAqG+UaQAAAAABAAAAAAAAAOgDAAAAAAAAAQAAAGMAAAAAAAAAQEIPAAAAAAAQJwABaAAQJwECGhAnAgMLAwIAAAEJAym/lQcqT78E33F1k+c4vMwhJygVwkcagNn59VWw1IQlASQEFwAoAV0T1DATNHXpX/sFnj+G3qAzHNFlFcd3JmJW5UXLSEmBB7Swr6yxXbIDrlmz4k8+KpZXHoZ2stouSFQQDE0nTzzoyEvg3OKGj8kaqe0DAQYEAwMCAA==";

    #[test]
    fn test_decode_real_transaction() {
        let tx = decode_transaction_base64(REAL_TX_BASE64).unwrap();

        // 2 signatures (one real, one placeholder)
        assert_eq!(tx.signatures.len(), 2);
        assert!(tx.signatures[0].starts_with("4vBpXi9zG"));

        let msg = &tx.message;
        assert_eq!(msg.version, MessageVersion::V0);
        assert_eq!(msg.header.num_required_signatures, 2);
        assert_eq!(msg.header.num_readonly_signed_accounts, 1);
        assert_eq!(msg.header.num_readonly_unsigned_accounts, 6);
        assert_eq!(msg.account_keys.len(), 14);

        // Should have multiple instructions
        assert!(msg.instructions.len() >= 2);

        // Find the instruction containing embedded fill_exact_in params
        // (called via CPI from Jupiter, not a standalone instruction)
        let fill_ix = msg
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find embedded fill_exact_in params in a Jupiter instruction");

        let (fill, analysis) = fill_ix.fill.as_ref().unwrap();

        // Verify decoded fill parameters
        assert!(fill.amount_in_atoms > 0);
        assert!(fill.params.tick_size_qpb > 0);
        assert!(fill.params.lot_size_base > 0);
        assert!(!fill.params.levels.is_empty());

        // Verify analysis ran successfully
        assert!(analysis.amount_out_atoms > 0);
        assert!(analysis.vwap_ticks > 0);
        assert!(analysis.levels_consumed > 0);

        // Print the full decoded output for manual inspection
        println!("{}", tx);
    }

    #[test]
    fn test_decode_real_message_only() {
        // Extract the message portion from the same real transaction.
        // Wire format: compact-u16(num_sigs) + num_sigs × 64-byte sigs + message.
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let tx_bytes = STANDARD.decode(REAL_TX_BASE64).unwrap();
        // num_sigs = 2 → compact-u16 encodes as single byte 0x02
        // skip: 1 (compact header) + 2 * 64 (signatures) = 129 bytes
        let msg_bytes = &tx_bytes[129..];
        let msg_b64 = STANDARD.encode(msg_bytes);

        let msg = decode_message_base64(&msg_b64).unwrap();

        assert_eq!(msg.version, MessageVersion::V0);
        assert_eq!(msg.account_keys.len(), 14);

        // Same fill should be found
        let fill_ix = msg
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find embedded fill_exact_in params");

        let (fill, analysis) = fill_ix.fill.as_ref().unwrap();
        assert!(fill.amount_in_atoms > 0);
        assert!(fill.params.tick_size_qpb > 0);
        assert!(analysis.levels_consumed > 0);

        println!("{}", msg);
    }

    // ---- Second real transaction: A3QA token → USDC fill ----

    /// Real transaction (message hash) from Solana mainnet: taker sells A3QA token for USDC.
    /// Tx sig: 2gHWdMvw1bYLq63P9FQ3GfhvVBQjibdZtkPRCiQ9r2wS4fkJphsMzN5K9gTohhcFxtyynytqWrcLXtPxZrjbZq3q
    /// On-chain log: side=Ask, amount_in=4000000, amount_out=799960, vwap_ticks=199990
    /// This is the pre-signing form (second signature is zeroed out).
    const REAL_TX2_BASE64: &str = "AlPlBOM0/PJtMdGe0Umk2ZL+l83VLIZlnto+clZr77+LZpMHjFOMhyn8d9paVwW2MUB5yfiVF9rQoqDX+HdVZgQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAIBBQuWmpufqjVYZfnyuFyDIYJfGWS2mAfjxKCUSfgP6ocw/3bk5le5izsp4223iXZnE4grlusUmzXQL4KHUihpkIa4FRxK4rsywvjpGrtIXvcwlTqIfhWBjuSSKYumfXogybHMphZItU08AEZ6ahGHYgLn6pzxnUhwhDZJYTR1tfeVfdtjecj/osn4Wsppt6OzLN+qBRRBwfnZU+16g1g0LNZ01wzdbD2DjJmsVeUGo84syt1HfCXTCpDzC/xmIqVkziGGVoqJoXHQv3NrqnA8ud4Fpk5exDk5GSvBfPxu23BzewMGRm/lIRcy/+ytunLDm+e8jOW7xfcSayxDmzpAAAAACeTUjwGeOKmNaN8QhVjkQuAyPqrOdqWbR2vIsdN/sy4EedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwan1RcYe9FmNdrUBFX9wsDBJMaPIVZ1pdu6y18IAAAASNIEZ4DY6P0pwzwN89l81A/PZUIvzSKvmhMEoq6GJccDBwAFAjrVAAAHAAkDQCsAAAAAAAAJFwACAwYMDg4JCwkIAAECAwQFBgwODgoNWLtk+swxxK8UAAk9AAAAAADYNAwAAAAAADIAAAAAAAEAAAB4ASwAAACVbp1pAAAAAAEAAAAAAAAAQEIPAAAAAAABAAAANg0DAAAAAADoAwAAAAAAABAnAAEBKb+VBypPvwTfcXWT5zi8zCEnKBXCRxqA2fn1VbDUhCUABAAoAhQ=";

    #[test]
    fn test_decode_real_transaction_2() {
        let tx = decode_transaction_base64(REAL_TX2_BASE64).unwrap();

        // 2 signatures (first real, second zeroed placeholder)
        assert_eq!(tx.signatures.len(), 2);

        let msg = &tx.message;
        assert_eq!(msg.version, MessageVersion::V0);
        assert_eq!(msg.header.num_required_signatures, 2);
        assert_eq!(msg.header.num_readonly_signed_accounts, 1);
        assert_eq!(msg.header.num_readonly_unsigned_accounts, 5);
        assert_eq!(msg.account_keys.len(), 11);

        // 3 instructions: ComputeBudget × 2 + Jupiter route
        assert_eq!(msg.instructions.len(), 3);

        // Find the instruction containing embedded fill_exact_in params
        let fill_ix = msg
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find embedded fill_exact_in in Jupiter route instruction");

        let (fill, analysis) = fill_ix.fill.as_ref().unwrap();

        // On-chain log says: side=Ask, amount_in=4000000, amount_out=799960, vwap_ticks=199990
        assert_eq!(fill.taker_side, Side::Ask);
        assert_eq!(fill.amount_in_atoms, 4_000_000);
        assert_eq!(fill.params.tick_size_qpb, 1);
        assert_eq!(fill.params.lot_size_base, 1_000_000);
        assert_eq!(fill.params.levels.len(), 1);
        assert_eq!(fill.params.levels[0].px_ticks, 199_990);
        assert_eq!(fill.params.levels[0].qty_lots, 1_000);

        assert_eq!(analysis.taker_side, Side::Ask);
        assert_eq!(analysis.amount_in_atoms, 4_000_000);
        assert_eq!(analysis.amount_spent_atoms, 4_000_000);
        assert_eq!(analysis.amount_out_atoms, 799_960);
        assert_eq!(analysis.vwap_ticks, 199_990);
        assert_eq!(analysis.levels_consumed, 1);
        assert_eq!(analysis.total_lots_filled, 4);

        // Print full decoded output for inspection
        println!("{}", tx);
    }

    // ---- Validation tests ----

    #[test]
    fn test_fill_exclusivity_real_tx() {
        let tx = decode_transaction_base64(REAL_TX2_BASE64).unwrap();
        let msg = &tx.message;

        // Maker accounts from the fill_exact_in (positions 4 and 5 in the IDL):
        let maker_base = "FmQGEXvc2houbBgw1HVPYf7gA6JBxzhCMUQWK1tky7B9";
        let maker_quote = "FUU2uSdMnTVcZWesD5Fen8AJUs7mSMdnM6qKMUCnqVw6";
        let fill_authority = "917Yp1mesMs14d32kDwH4uNocdhuB67QzzaYKezkjy4B";

        // Each maker account should appear exclusively in the fill instruction.
        let report = check_fill_exclusivity(msg, maker_base);
        assert!(report.is_exclusive(), "maker_base: {}", report);
        assert_eq!(report.fill_instruction_indices, vec![2]);

        let report = check_fill_exclusivity(msg, maker_quote);
        assert!(report.is_exclusive(), "maker_quote: {}", report);

        let report = check_fill_exclusivity(msg, fill_authority);
        assert!(report.is_exclusive(), "fill_authority: {}", report);

        // Convenience: check all at once.
        assert!(all_exclusive(
            msg,
            &[maker_base, maker_quote, fill_authority]
        ));
    }

    #[test]
    fn test_fill_exclusivity_non_existent_key() {
        let tx = decode_transaction_base64(REAL_TX2_BASE64).unwrap();
        let msg = &tx.message;

        let report = check_fill_exclusivity(msg, "11111111111111111111111111111111");
        assert!(!report.is_exclusive());
        assert!(report.fill_instruction_indices.is_empty());
        assert!(report.non_fill_instruction_indices.is_empty());
    }

    #[test]
    fn test_fill_exclusivity_user_key_not_exclusive() {
        // The user (taker) account appears in the Jupiter route instruction
        // which contains the fill, so it IS exclusive in this single-route tx.
        // But it also appears as a signer which is fine.
        let tx = decode_transaction_base64(REAL_TX2_BASE64).unwrap();
        let msg = &tx.message;

        let user = "B8ttfFCJRyJivDLn19Q6uvndVCssTwkokLAgz22vyo1Q";
        let report = check_fill_exclusivity(msg, user);
        // The user shows up in the Jupiter fill instruction (ix 2) only.
        assert!(report.is_exclusive(), "user: {}", report);
    }

    // ---- FillMints tests ----

    #[test]
    fn test_fill_mints_from_base_quote_bid() {
        // Bid: taker pays quote, receives base
        let m = FillMints::from_base_quote("BASE".into(), "QUOTE".into(), Side::Bid);
        assert_eq!(m.input_mint, "QUOTE");
        assert_eq!(m.output_mint, "BASE");
        assert_eq!(m.base_mint, "BASE");
        assert_eq!(m.quote_mint, "QUOTE");
    }

    #[test]
    fn test_fill_mints_from_base_quote_ask() {
        // Ask: taker pays base, receives quote
        let m = FillMints::from_base_quote("BASE".into(), "QUOTE".into(), Side::Ask);
        assert_eq!(m.input_mint, "BASE");
        assert_eq!(m.output_mint, "QUOTE");
        assert_eq!(m.base_mint, "BASE");
        assert_eq!(m.quote_mint, "QUOTE");
    }

    #[test]
    fn test_fill_mints_from_input_output_bid() {
        // Bid: input=quote, output=base → derive base/quote
        let m = FillMints::from_input_output("QUOTE".into(), "BASE".into(), Side::Bid);
        assert_eq!(m.base_mint, "BASE");
        assert_eq!(m.quote_mint, "QUOTE");
    }

    #[test]
    fn test_fill_mints_from_input_output_ask() {
        // Ask: input=base, output=quote → derive base/quote
        let m = FillMints::from_input_output("BASE".into(), "QUOTE".into(), Side::Ask);
        assert_eq!(m.base_mint, "BASE");
        assert_eq!(m.quote_mint, "QUOTE");
    }

    #[test]
    fn test_fill_mints_embedded_real_tx_2() {
        // REAL_TX2: route_v2 single-step, Ask, taker sells A3QA for USDC.
        // Mints are read directly from the embedded fill_exact_in account
        // block (positions 6 and 7), not from the route's source/destination.
        let tx = decode_transaction_base64(REAL_TX2_BASE64).unwrap();
        let fill_ix = tx
            .message
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find fill");

        let mints = fill_ix.fill_mints.as_ref().expect("fill_mints populated");

        // Base = A3QA (static key); quote (USDC) sits in the ALT and surfaces
        // as a placeholder pre-RPC-resolution.
        assert_eq!(
            mints.base_mint,
            "A3QAoKnf3jFcCfTGvEpE7KVBMZqXQJwvwt6Uc4UExkDp"
        );
        assert!(
            mints.quote_mint.starts_with("LookupReadonly["),
            "expected lookup placeholder, got {}",
            mints.quote_mint
        );
        // taker_side=Ask → input = base, output = quote
        assert_eq!(mints.input_mint, mints.base_mint);
        assert_eq!(mints.output_mint, mints.quote_mint);
    }

    #[test]
    fn test_fill_mints_embedded_real_tx_1() {
        // REAL_TX1 is a 3-leg parallel split. The instructions sysvar lives in
        // an ALT here, exercising the lookup-placeholder branch of the block
        // validator.
        let tx = decode_transaction_base64(REAL_TX_BASE64).unwrap();
        let fill_ix = tx
            .message
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find fill");
        assert!(fill_ix.fill_mints.is_some());
    }

    /// USDT → USDC fill embedded as the first leg of a 2-step Jupiter route
    /// (USDT → USDC → SOL). The route's destination_mint is wSOL but the
    /// RFQ leg's actual base_mint is USDC — verifies the resolver reads
    /// from the embedded fill block, not the route's source/dest.
    const REAL_TX3_BASE64: &str = "AhcA6+ZC1kWvFdDjlDTqW2dIcjvtninQs4TPOCM8AYtjf9P+8Jxj6ljR3Zvlt3d61kPMcorJZ/7C6WEIHZGJ0AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAIBBg0CiOE7jnRYBkI7XU7ZH3QKA/aGNdP+1qa0dVADNmPABsOAvIYcfaKJKbntsRZhkmE5z6xpVI3UbVOSlAkYpnljM9OKLuF/OLQ/lmpeTEGZEe43VqDC0mQoim65HePWwfRQzh2vMinR51RMtSBehc2T2HaUHHRVhiAQUKpftXyPLHuLntEEYI0LF0BkR8R+qu33k4W2mmtaihzBna5tNajvwv+YF9CXF3oPUYQ0JkGwRP3YyqgVzK9NAQ3Wwl0Mu93o2uD9m/1rpADKOjC3fE4vE5OOe1sMPA4ZuYhhVxvpFoyXJY9OJInxuz0QKRSODYMLWhOZ2v8QhASOe9jb6fhZAwZGb+UhFzL/7K26csOb57yM5bvF9xJrLEObOkAAAADOAQ5gr+2yJxe9YxkvVBRaP5ZaM7uC0scCnrLOHiCCZAnk1I8BnjipjWjfEIVY5ELgMj6qznalm0dryLHTf7MuBHnVW/IxwG7udMVuzmgVB/2xst6j9I5RArHNola8E48G3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqTga1+fLePEE5rAEWLsmL0x75DCVIsTabIoie7gTRhGKBggABQIYEgMACAAJA+NnBgAAAAAACwUGABUMEQmT8Xtk9ISudv4HBgADABMRDAEBCyUAAgYJFQwMCxILDQoAAQMCBAUTCQwMFxgWEAAODwYDFRMMDBcUXrtk+swxxK8UmJKYAAAAAADnua8GAAAAABYAAgAAAAIAAAB4ACwAAAAOwftpAAAAAAEAAAAAAAAAQEIPAAAAAAABAAAAcEEPAAAAAAAKAAAAAAAAABAnAAFZABAnAQIMAwYAAAEJAim/lQcqT78E33F1k+c4vMwhJygVwkcagNn59VWw1IQlAScFEwAoARd5QE4t+Dvx0UlyGT++v3V9s/1gQI0crEMfwbwNXZBmFgPfF9sDFuHc";

    #[test]
    fn test_fill_mints_embedded_multi_step_route() {
        // 2-step sequential route, USDT → USDC → SOL. RFQ is leg 1 (USDT→USDC).
        // Pre-RPC: USDT is a static key, USDC is in an ALT.
        let tx = decode_transaction_base64(REAL_TX3_BASE64).unwrap();
        let fill_ix = tx
            .message
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find fill");

        let mints = fill_ix.fill_mints.as_ref().expect("fill_mints populated");

        // USDT (Tether) is a static account key in this tx.
        assert_eq!(
            mints.quote_mint,
            "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"
        );
        // USDC lives in the ALT — placeholder until RPC resolution.
        assert!(
            mints.base_mint.starts_with("LookupReadonly["),
            "expected lookup placeholder for USDC base_mint, got {}",
            mints.base_mint
        );
        // taker_side=Bid → input = quote (USDT), output = base (USDC)
        assert_eq!(mints.input_mint, mints.quote_mint);
        assert_eq!(mints.output_mint, mints.base_mint);
    }

    #[test]
    fn test_embedded_fill_accounts_are_labeled() {
        // Verify the 11 fill_exact_in accounts get labeled inside the parent
        // (Jupiter route) instruction.
        let tx = decode_transaction_base64(REAL_TX2_BASE64).unwrap();
        let fill_ix = tx
            .message
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find fill");

        let labels: Vec<&str> = fill_ix
            .accounts
            .iter()
            .filter_map(|a| a.label.as_deref())
            .collect();
        assert_eq!(labels.len(), FILL_EXACT_IN_ACCOUNT_COUNT);
        assert_eq!(labels[0], "user");
        assert_eq!(labels[1], "fill_authority");
        assert_eq!(labels[6], "base_mint");
        assert_eq!(labels[7], "quote_mint");
        assert_eq!(labels[10], "instructions_sysvar");
    }

    #[test]
    fn test_fill_exclusivity_multi() {
        let tx = decode_transaction_base64(REAL_TX2_BASE64).unwrap();
        let msg = &tx.message;

        let keys = [
            "FmQGEXvc2houbBgw1HVPYf7gA6JBxzhCMUQWK1tky7B9",
            "FUU2uSdMnTVcZWesD5Fen8AJUs7mSMdnM6qKMUCnqVw6",
        ];
        let reports = check_fill_exclusivity_multi(msg, &keys);
        assert_eq!(reports.len(), 2);
        assert!(reports.iter().all(|r| r.is_exclusive()));
    }

    /// 4-step Jupiter route_v2 where the RFQ leg sits mid-chain
    /// (input_index = 2, fed by an upstream RaydiumClmm step's output).
    const REAL_TX4_BASE64: &str = "AuqP2BRrYGvIj/W7IBvvHyVEx/Wcz0Wm0CaCKATCWu8p6pZGezgaDdrWsRow7IJ0eqSa1uAb9AngHuRSL6NwWQkVNwTug9FqMeZW3bHCLaKYjnEF/eg8qCJEkJGz2O4sP3NRjTumDDEAI1m5Hn47x1FTmIyKYB7KxvwfjRgrQwsDgAIBBhIXivRTIfL2uclOoIDaVlBRpbZc0/1vt/0tfj2F5/QWGcOAvIYcfaKJKbntsRZhkmE5z6xpVI3UbVOSlAkYpnljIX3c5GeIjZQcJMPv+W9LORUt/YNNiYuyDC5Va3cg7p1YQqAVCY/gBTNjDhUjvsFmo9dtqaQsQOlQPBLYpWjpplzPNY+qGCNzKDGfvcpquLUNmZGsvm3fcwmDcTz0Gcq8e4ue0QRgjQsXQGRHxH6q7feThbaaa1qKHMGdrm01qO+PPTr71nYMqeKudzQr8hwkyc5nl/ZBHKqHn+WoZzDRHotko3tr+j4T3bf58aiXZfUv+dBChleWTWDBjxOB/BORjV2jRG3lLHpikQscgXyH/fbz98hlrGb2KJ2mI0wuGJGvvWBkXYRoVQrowF+Qj0uxxf4glrSiRkPLjfRw11+Zl9NvPqZ7il6X2pDA57IZpJ4hICBvo5Lb9BnL7zRf9rw86zUZxt7gqJneytRCD2WD/g7IaVAM5kiJ4AsS6/qnnbwDBkZv5SEXMv/srbpyw5vnvIzlu8X3EmssQ5s6QAAAALw1n9V4Uh7ztRZFBlw3IE5nZpsKp/d96y1JFGAScORfCeTUjwGeOKmNaN8QhVjkQuAyPqrOdqWbR2vIsdN/sy4EedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwan1RcYe9FmNdrUBFX9wsDBJMaPIVZ1pdu6y18IAAAABt324ddloZPZy+FGzut5rBy0he1fWzeROoz1hX7/AKkwCSNhP6lRiaATYvHGYgVAlrUDkQIVTEOdi7Tlpw7JAwUMAAUCalgEAAwACQMEEQQAAAAAAA8FCQAjER8Jk/F7ZPSErnb3Dz4ACgYqDRERDyAPCyURESQAEiojChQJFQcCCBMpACgaCgMZGx4RHRwPDgABCQMEBSMhEREQJxEWJhcYCQYAImm7ZPrMMcSvFAAw7326AgAAY2YXEwAAAABkAAoAAAAEAAAALwEANxEAAxrZFQACeAAsAAAAgKwBagAAAAABAAAAAAAAAICWmAAAAAAAAQAAAJ6KDgAAAAAACgAAAAAAAAAQJwIDaRAnAwQRAwkAAAEJBCm/lQcqT78E33F1k+c4vMwhJygVwkcagNn59VWw1IQlAAUTACgBF3e1Hwi9jFUGJi6HHxxyBS+upOGzy/UKPjOBDttDtzudBFg+WTsCNj3FowmaMdRo5uFCzdySENNYqALhvJ3Zs7YaaCHuvsvC5AMoJCcCGRf10YNGzrwO3aesPuetei3vDGJDYVRvEkiu3YQxk0PsAwZUUFVvcFcDAwQc";

    #[test]
    fn test_mid_chain_rfq_leg_does_not_inherit_route_in_amount() {
        let tx = decode_transaction_base64(REAL_TX4_BASE64).unwrap();
        let fill_ix = tx
            .message
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find embedded fill in Jupiter route_v2");

        let (fill, analysis) = fill_ix.fill.as_ref().unwrap();

        assert_eq!(fill.taker_side, Side::Bid);
        assert_eq!(fill.amount_in_atoms, 0);
        assert_eq!(fill.params.tick_size_qpb, 1);
        assert_eq!(fill.params.lot_size_base, 10_000_000);
        assert_eq!(fill.params.levels.len(), 1);
        assert_eq!(fill.params.levels[0].px_ticks, 952_990);
        assert_eq!(fill.params.levels[0].qty_lots, 10);

        assert_eq!(analysis.amount_in_atoms, 0);
        assert_eq!(analysis.amount_spent_atoms, 0);
        assert_eq!(analysis.amount_out_atoms, 0);
        assert_eq!(analysis.levels_consumed, 0);
        assert_eq!(analysis.total_lots_filled, 0);
    }

    /// Tx sig: 3F35sV3LmhrtqRTUUMnEAiSdBf8e4pjrjb6h29xpU7ChYr3sTAhM1bYyPJozPNfHV5qe4BfTCc7HPD8WFNNjDrwD
    const REAL_TX5_BASE64: &str = "AnAjV0b0u7heGiPJNLiygSyqeHvahqWK0nlzB3TMRxJJVLOX9wxc4nKLaa5aYUV9YcDIy3pvTPLkvtPAQrr8PgwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAIBBRHX6YjW56lQwn+O17i7jjcjzC1eF63s8vP6w8CsNV9uBMOAvIYcfaKJKbntsRZhkmE5z6xpVI3UbVOSlAkYpnljMbgl6izdcZKG3L7tA0dDY1cE9t0qQ3lgLZbIED0jQNsyi8boMOin2qmKB+86DcC5AmP6yoMNUdE878Dp/eeeej/ze4cazHXK6oSe4+/WDNxWVmKTxr2GfhW5j7S3Sx0OTEn8xSw6B4UnG4+WeCMjbgEvhnAFj0oscjRbCk1IVqVczzWPqhgjcygxn73Kari1DZmRrL5t33MJg3E89BnKvHuLntEEYI0LF0BkR8R+qu33k4W2mmtaihzBna5tNajvuEu3dNr1R3TS8gwOeHtFxlE+X1aRiLlCabfFf4Xu6dvCkhdM+2kiT54U4225DhuscXLXJ66a2pXi7fir1acrx91ZL55tRxnYTx7WNVa7qyePbVB8wqdAssrLg9oEF+/F8Yfsh9H3Rcs6AzhKJqae2gyi0aoPQeQkFjd+kf9bXTEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMGRm/lIRcy/+ytunLDm+e8jOW7xfcSayxDmzpAAAAACeTUjwGeOKmNaN8QhVjkQuAyPqrOdqWbR2vIsdN/sy4EedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwbd9uHXZaGT2cvhRs7reawctIXtX1s3kTqM9YV+/wCpcaE+wD9JyP4GcMx1Xto/W7wWXcZkMd7JG5h6mbCJ+I4GDQAFAu94AwANAAkDAAAAAAAAAAAPBQIAIBAMCZPxe2T0hK52/w9GAAoFHiIQEA8dDw4AAQIKBgcgHhAQIygAKRoIChscGxwcECMlIQ8nECEmABcgJQIZCBgDBAkWJBEMExUSFAIFABAgECIjH2q7ZPrMMcSvFANjnwIAAAAAD2CgAgAAAAAAAAAAAAAEAAAAeAAsAAAAGXYVagAAAAABAAAAAAAAAICWmAAAAAAAAQAAAHMHDQAAAAAAMgAAAAAAAADqJQADaAAmAQACLwAAECcCA5oQJwMEEAMCAAABCQwCAAsMAgAAAPEQAAAAAAAABCm/lQcqT78E33F1k+c4vMwhJygVwkcagNn59VWw1IQlAAUAKAIXFVEeco22aHXe6ued3Jm1CXy2P74xNcI96BRXmWGT5tpVBY6PjZOLA5KRjMJRwDIR3Tbag/HLEdZxTPfqLWdCCyd0nco65bHdIoy/BCcoLzADMiYs1fUEn7nGinHgFz2Y1Gr0z2SQYAtcxgTamGgKyJ2kcBgD3d9EAkne";

    #[test]
    fn test_unknown_later_swap_variant_does_not_poison_rfq_leg() {
        let tx = decode_transaction_base64(REAL_TX5_BASE64).unwrap();
        let fill_ix = tx
            .message
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should still find the embedded RFQ leg despite unknown step 3 variant");

        let (fill, analysis) = fill_ix.fill.as_ref().unwrap();

        assert_eq!(fill.taker_side, Side::Bid);
        // route_in (44,000,003) × bps (9706) / 10_000 — the amount that
        // actually flows into the RFQ leg, not the route's total input and
        // not the ~188-billion the buggy generic scanner used to invent.
        assert_eq!(fill.amount_in_atoms, 42_706_402);
        assert_eq!(fill.params.tick_size_qpb, 1);
        assert_eq!(fill.params.lot_size_base, 10_000_000);
        assert_eq!(fill.params.levels.len(), 1);
        assert_eq!(fill.params.levels[0].px_ticks, 853_875);
        assert_eq!(fill.params.levels[0].qty_lots, 50);

        // Matches the on-chain `Fill executed` log.
        assert_eq!(analysis.amount_spent_atoms, 42_693_750);
        assert_eq!(analysis.amount_out_atoms, 500_000_000);
        assert_eq!(analysis.vwap_ticks, 853_875);
        assert_eq!(analysis.levels_consumed, 1);
        assert_eq!(analysis.total_lots_filled, 50);
    }

    /// Tx sig: 2p167rbza2AjoKXCMeA2SauuFYsaLK4RNy9sZD5DJpPD9KMhiT3BZM43TYPm6u22AgYLLiSBbM6W1YkwCxmoapH8
    const REAL_TX6_BASE64: &str = "AlqMoKcVaT8PWWVIcc/4IKUPOW69+K+MkY2Uv6D7JlvDSUCanNL1hpXHyHpJMxkbodh+/kzjRnjVgRWe8Kz8yAsAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAIBBg0GXGniedCvdfyCSnwJK0Ofs2JmBoQNlbtMERORE6wYC8OAvIYcfaKJKbntsRZhkmE5z6xpVI3UbVOSlAkYpnljXM81j6oYI3MoMZ+9ymq4tQ2Zkay+bd9zCYNxPPQZyrx7i57RBGCNCxdAZEfEfqrt95OFtpprWoocwZ2ubTWo745BueiehHmGANK0I7QeI6KB1PrgzpJYnnbwCocrdNFP9bH3ut15aIgB5AlyMk70KmcRATrjkYJUhy+FHSQVD5QOexMv32RdwGY100l7UhFIP6wbtEZJB8Sr4EhLbPWZvwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwZGb+UhFzL/7K26csOb57yM5bvF9xJrLEObOkAAAAAJ5NSPAZ44qY1o3xCFWORC4DI+qs52pZtHa8ix03+zLvzRPaBBVZATz0rroahO1wI61P8hqEdH/zsFqzD6ggfDBHnVW/IxwG7udMVuzmgVB/2xst6j9I5RArHNola8E48G3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqdFqDUwK/qLHqBCBpQbV9uEciXPAiwNtHiDbubxSu8YHBggABQLQcQUACAAJA3kQBwAAAAAABwIABQwCAAAA6RI10QAAAAALBQUAGgwHCZPxe2T0hK52/wtFAAUEGhgMDAsXCw0eAA8FBhAOGh0cGx8MDAoeABUGBBQWHRghGx8MDAoJAAEFBAIDGhgMDB8eABMFBBESGhggGx8MDAoZartk+swxxK8U+fQV0QAAAABvPeQRAAAAADIAAgAAAAQAAACXAHMSAAGXABAnAQR4ASwAAACMeRVqAAAAAAEAAAAAAAAAgJaYAAAAAAABAAAAnRENAAAAAAAyAAAAAAAAAAIDAASXAJsRAAQMAwUAAAEJBCm/lQcqT78E33F1k+c4vMwhJygVwkcagNn59VWw1IQlASUEACgCFzHdwSMw/1NhB0GAQ4JsVDmT9kfqH79wtmLGV4ArowmxA2ajaQVoaqRnZbUOH7fEvxv51OIjUNwzr2F9cdBp6WMK3CKAhTB7GSlRA+rv8QHy3UDLjSWiR7/tOMfA+EoPNrP1tz3kWPHSsN0YqYH6sKkDEhcUARM=";

    #[test]
    fn test_parallel_split_route_applies_bps_to_rfq_leg() {
        let tx = decode_transaction_base64(REAL_TX6_BASE64).unwrap();
        let fill_ix = tx
            .message
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find embedded RFQ leg in 4-step parallel split");

        let (fill, analysis) = fill_ix.fill.as_ref().unwrap();

        assert_eq!(fill.taker_side, Side::Ask);
        // route_in (3,507,877,113) × bps (770) / 10_000 = 270,106,537.
        assert_eq!(fill.amount_in_atoms, 270_106_537);
        assert_eq!(fill.params.tick_size_qpb, 1);
        assert_eq!(fill.params.lot_size_base, 10_000_000);
        assert_eq!(fill.params.levels.len(), 1);
        assert_eq!(fill.params.levels[0].px_ticks, 856_477);
        assert_eq!(fill.params.levels[0].qty_lots, 50);

        // Matches the on-chain `Fill executed` log: 27 lots × 10M base atoms,
        // 27 lots × 856,477 quote atoms.
        assert_eq!(analysis.amount_spent_atoms, 270_000_000);
        assert_eq!(analysis.amount_out_atoms, 23_124_879);
        assert_eq!(analysis.vwap_ticks, 856_477);
        assert_eq!(analysis.levels_consumed, 1);
        assert_eq!(analysis.total_lots_filled, 27);
    }

    /// Tx sig: 58y3YPbpdeD6Py5VjjZt1NWvxvhgai9Tw2i4EZkFTiGWGWA6djY1w1pQPbqvP8z21sWN8qZKSo7pxEVs6K22Johp
    const REAL_TX7_BASE64: &str = "As7wE/epjlX1pS0WGiEb91phmEM7qgPThY0WgbgT3iT5DPrG87Gpcxqmz/fZ2xQ4dv6/sFVChYrr0LIh3kGqUA8/pecuQWJ7iKWuqaOBcRyoIKoqA5uJlY+QImK4t2GeSaflDzctcft2c4IvLBlxpag17ZMWazMqz2JBGgd1SmoFgAIBBRDX6YjW56lQwn+O17i7jjcjzC1eF63s8vP6w8CsNV9uBMOAvIYcfaKJKbntsRZhkmE5z6xpVI3UbVOSlAkYpnljNaQ2AtK4l+IcRXn2ydERhF3PwkoQz4S+L5MCsM2x86oxuCXqLN1xkobcvu0DR0NjVwT23SpDeWAtlsgQPSNA20xJ/MUsOgeFJxuPlngjI24BL4ZwBY9KLHI0WwpNSFalXM81j6oYI3MoMZ+9ymq4tQ2Zkay+bd9zCYNxPPQZyryANBhCc5KMkz14UHkwZqhpHsA0BpmGCVHXQKwoAELYbXuLntEEYI0LF0BkR8R+qu33k4W2mmtaihzBna5tNajviPH/o6Lf5he9xONXMlGjIuP8roHlpFc5DmR1HACkZeKg80OHuuYEKqH5J9guy/ZYT5HW7JwaFFRk3GveYoXrV91ZL55tRxnYTx7WNVa7qyePbVB8wqdAssrLg9oEF+/FAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADBkZv5SEXMv/srbpyw5vnvIzlu8X3EmssQ5s6QAAAAAnk1I8BnjipjWjfEIVY5ELgMj6qznalm0dryLHTf7MuBHnVW/IxwG7udMVuzmgVB/2xst6j9I5RArHNola8E48G3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqXptQ8QgmAoVIMbdzGNCR+m9pOUpyXDcQ8wSdDENjksvBgwABQIo6AgADAAJAwAAAAAAAAAADgUDACIPCwmT8Xtk9ISudv8ORAAECiMgDw8OHw4kAAASCwQKExQPIw8gEBEmFQsWGBcZBAMADyIPIyUnACgaBAMeGx0PCRwCBg4NAAEDCgUHIiAPDyUhc7tk+swxxK8Um4r3AQAAAAA0K/cBAAAAAAAAAAAAAAMAAAB0AcoPAAOSAgAAAAtGFwAABAAAAAAoBEYXAAJ4ASwAAACBgRVqAAAAAAEAAAAAAAAAgJaYAAAAAAABAAAATQkNAAAAAAAyAAAAAAAAABAnAgMPAwMAAAEJCwIACAwCAAAAEQ8AAAAAAAAEKb+VBypPvwTfcXWT5zi8zCEnKBXCRxqA2fn1VbDUhCUABAAoAhc32dQnS2zFLlchy1ks/hE9pmWD17IjlWmWM7Bf6Ogq+QU+P4+GiwIfOVEeco22aHXe6ued3Jm1CXy2P74xNcI96BRXmWGT5tpVBY6PjZOLApGMeS0YM07Po4EiEOOV4Ah6w0iCKUuQPkq5T3mRQbXSWb0F7bWz7rECsOw=";

    #[test]
    fn test_mid_chain_rfq_with_unknown_earlier_variant() {
        let tx = decode_transaction_base64(REAL_TX7_BASE64).unwrap();
        let fill_ix = tx
            .message
            .instructions
            .iter()
            .find(|ix| ix.fill.is_some())
            .expect("should find the embedded RFQ leg via byte-scan despite unknown variant 146");

        let (fill, analysis) = fill_ix.fill.as_ref().unwrap();

        assert_eq!(fill.taker_side, Side::Ask);
        // Mid-chain leg — input only known at runtime, so we honestly return 0.
        assert_eq!(fill.amount_in_atoms, 0);
        assert_eq!(fill.params.tick_size_qpb, 1);
        assert_eq!(fill.params.lot_size_base, 10_000_000);
        assert_eq!(fill.params.levels.len(), 1);
        assert_eq!(fill.params.levels[0].px_ticks, 854_349);
        assert_eq!(fill.params.levels[0].qty_lots, 50);

        // Zero amounts follow from amount_in=0 — sweep has nothing to consume.
        assert_eq!(analysis.amount_spent_atoms, 0);
        assert_eq!(analysis.amount_out_atoms, 0);
        assert_eq!(analysis.levels_consumed, 0);
        assert_eq!(analysis.total_lots_filled, 0);
    }

    // ---- platform_fee_bps extraction ----

    const ROUTE_DISC: [u8; 8] = [229, 23, 203, 151, 122, 227, 173, 42];
    const SHARED_ACCOUNTS_ROUTE_V2_DISC: [u8; 8] = [209, 152, 83, 147, 124, 254, 216, 233];
    const ROUTE_V2_DISC: [u8; 8] = [187, 100, 250, 204, 49, 196, 175, 20];

    #[test]
    fn test_extract_platform_fee_v1_ignores_trailing() {
        let mut data = Vec::new();
        data.extend_from_slice(&ROUTE_DISC);
        // route_plan: Vec<RoutePlanStep> with one Saber (unit variant, tag 0) step
        data.extend_from_slice(&1u32.to_le_bytes()); // route_plan len
        data.push(0); // Swap::Saber
        data.push(100); // percent
        data.push(0); // input_index
        data.push(1); // output_index
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // in_amount
        data.extend_from_slice(&990_000u64.to_le_bytes()); // quoted_out_amount
        data.extend_from_slice(&50u16.to_le_bytes()); // slippage_bps
        data.push(25); // platform_fee_bps (u8) — the value we want
                       // trailing RemainingAccountsInfo: one slice {accounts_type, length}
        data.extend_from_slice(&1u32.to_le_bytes()); // slices len
        data.push(7); // accounts_type
        data.push(3); // length — ends up as the final byte

        assert_eq!(extract_platform_fee_bps(&data), Some(25));
        // The old heuristic returned the last byte (3), proving the bug.
        assert_eq!(*data.last().unwrap(), 3);
    }

    #[test]
    fn test_extract_platform_fee_route_v2() {
        let mut data = Vec::new();
        data.extend_from_slice(&ROUTE_V2_DISC);
        data.extend_from_slice(&2_000_000u64.to_le_bytes()); // in_amount
        data.extend_from_slice(&1_980_000u64.to_le_bytes()); // quoted_out_amount
        data.extend_from_slice(&50u16.to_le_bytes()); // slippage_bps
        data.extend_from_slice(&30u16.to_le_bytes()); // platform_fee_bps (u16)
        data.extend_from_slice(&0u16.to_le_bytes()); // positive_slippage_bps
        data.extend_from_slice(&0u32.to_le_bytes()); // route_plan len = 0

        assert_eq!(extract_platform_fee_bps(&data), Some(30));
    }

    #[test]
    fn test_extract_platform_fee_shared_v2() {
        let mut data = Vec::new();
        data.extend_from_slice(&SHARED_ACCOUNTS_ROUTE_V2_DISC);
        data.push(1); // id
        data.extend_from_slice(&2_000_000u64.to_le_bytes()); // in_amount
        data.extend_from_slice(&1_980_000u64.to_le_bytes()); // quoted_out_amount
        data.extend_from_slice(&50u16.to_le_bytes()); // slippage_bps
        data.extend_from_slice(&30u16.to_le_bytes()); // platform_fee_bps (u16)
        data.extend_from_slice(&0u16.to_le_bytes()); // positive_slippage_bps
        data.extend_from_slice(&0u32.to_le_bytes()); // route_plan len = 0

        assert_eq!(extract_platform_fee_bps(&data), Some(30));
    }

    #[test]
    fn test_extract_platform_fee_non_route() {
        assert_eq!(extract_platform_fee_bps(&[0u8; 8]), None); // unknown disc
        assert_eq!(extract_platform_fee_bps(&[1, 2, 3]), None); // too short
    }
}
