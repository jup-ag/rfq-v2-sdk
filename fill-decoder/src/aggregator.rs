//! Jupiter aggregator instruction decoding.

use borsh::BorshDeserialize;

use crate::analysis::{analyze_fill, is_params_plausible};
use crate::types::{FillAnalysis, FillExactInInstruction, FillExactInParams, Side};

pub const JUPITER_PROGRAM_ID: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";

const ROUTE: [u8; 8] = [229, 23, 203, 151, 122, 227, 173, 42];
const ROUTE_WITH_TOKEN_LEDGER: [u8; 8] = [150, 86, 71, 116, 167, 93, 14, 104];
const EXACT_OUT_ROUTE: [u8; 8] = [208, 51, 239, 151, 123, 43, 237, 92];
const SHARED_ACCOUNTS_ROUTE: [u8; 8] = [193, 32, 155, 51, 65, 214, 156, 129];
const SHARED_ACCOUNTS_EXACT_OUT_ROUTE: [u8; 8] = [176, 209, 105, 168, 154, 125, 69, 62];
const SHARED_ACCOUNTS_ROUTE_WITH_TOKEN_LEDGER: [u8; 8] = [230, 121, 143, 80, 119, 159, 106, 170];
const ROUTE_V2: [u8; 8] = [187, 100, 250, 204, 49, 196, 175, 20];
const EXACT_OUT_ROUTE_V2: [u8; 8] = [157, 138, 184, 82, 21, 244, 243, 36];
const SHARED_ACCOUNTS_ROUTE_V2: [u8; 8] = [209, 152, 83, 147, 124, 254, 216, 233];
const SHARED_ACCOUNTS_EXACT_OUT_ROUTE_V2: [u8; 8] = [53, 96, 229, 202, 216, 187, 250, 24];

// Lenient: ignores trailing bytes. `borsh::from_slice` errors on them, but
// Jupiter routes append a `RemainingAccountsInfo` after the args.
fn deser<T: BorshDeserialize>(bytes: &[u8]) -> Option<T> {
    let mut slice = bytes;
    T::deserialize(&mut slice).ok()
}

#[derive(Debug, Clone, BorshDeserialize)]
#[allow(dead_code)]
struct RemainingAccountsSlice {
    accounts_type: u8,
    length: u8,
}

#[derive(Debug, Clone, BorshDeserialize)]
#[allow(dead_code)]
struct RemainingAccountsInfo {
    slices: Vec<RemainingAccountsSlice>,
}

#[derive(Debug, Clone, BorshDeserialize)]
enum JupSide {
    Bid,
    Ask,
}

/// One `JupiterRfqV2` step pulled out of a route plan:
/// `(side, fill_data, input_index, output_index, bps)`.
type RfqStep = (JupSide, Vec<u8>, u8, u8, u16);

#[derive(Debug, Clone, BorshDeserialize)]
#[allow(dead_code)]
enum CandidateSwap {
    HumidiFi {
        swap_id: u64,
        is_base_to_quote: bool,
    },
    TesseraV {
        side: JupSide,
    },
    HumidiFiV2 {
        swap_id: u64,
        is_base_to_quote: bool,
    },
}

// Layout drift in any variant breaks Borsh decoding of the whole
// route_plan — `decode_jupiter_rfq_step_indices` byte-scans as a fallback.
#[derive(Debug, Clone, BorshDeserialize)]
#[allow(dead_code)]
enum Swap {
    Saber,                    // 0
    SaberAddDecimalsDeposit,  // 1
    SaberAddDecimalsWithdraw, // 2
    TokenSwap,                // 3
    Sencha,                   // 4
    Step,                     // 5
    Cropper,                  // 6
    Raydium,                  // 7
    Crema {
        a_to_b: bool,
    }, // 8
    Lifinity,                 // 9
    Mercurial,                // 10
    Cykura,                   // 11
    Serum {
        side: JupSide,
    }, // 12
    MarinadeDeposit,          // 13
    MarinadeUnstake,          // 14
    Aldrin {
        side: JupSide,
    }, // 15
    AldrinV2 {
        side: JupSide,
    }, // 16
    Whirlpool {
        a_to_b: bool,
    }, // 17
    Invariant {
        x_to_y: bool,
    }, // 18
    Meteora,                  // 19
    GooseFX,                  // 20
    DeltaFi {
        stable: bool,
    }, // 21
    Balansol,                 // 22
    MarcoPolo {
        x_to_y: bool,
    }, // 23
    Dradex {
        side: JupSide,
    }, // 24
    LifinityV2,               // 25
    RaydiumClmm,              // 26
    Openbook {
        side: JupSide,
    }, // 27
    Phoenix {
        side: JupSide,
    }, // 28
    Symmetry {
        from_token_id: u64,
        to_token_id: u64,
    }, // 29
    TokenSwapV2,              // 30
    HeliumTreasuryManagementRedeemV0, // 31
    StakeDexStakeWrappedSol,  // 32
    StakeDexSwapViaStake {
        bridge_stake_seed: u32,
    }, // 33
    GooseFXV2,                // 34
    Perps,                    // 35
    PerpsAddLiquidity,        // 36
    PerpsRemoveLiquidity,     // 37
    MeteoraDlmm,              // 38
    OpenBookV2 {
        side: JupSide,
    }, // 39
    RaydiumClmmV2,            // 40
    StakeDexPrefundWithdrawStakeAndDepositStake {
        bridge_stake_seed: u32,
    }, // 41
    Clone {
        pool_index: u8,
        quantity_is_input: bool,
        quantity_is_collateral: bool,
    }, // 42
    SanctumS {
        src_lst_value_calc_accs: u8,
        dst_lst_value_calc_accs: u8,
        src_lst_index: u32,
        dst_lst_index: u32,
    }, // 43
    SanctumSAddLiquidity {
        lst_value_calc_accs: u8,
        lst_index: u32,
    }, // 44
    SanctumSRemoveLiquidity {
        lst_value_calc_accs: u8,
        lst_index: u32,
    }, // 45
    RaydiumCP,                // 46
    WhirlpoolSwapV2 {
        a_to_b: bool,
        remaining_accounts_info: Option<RemainingAccountsInfo>,
    }, // 47
    OneIntro,                 // 48
    PumpWrappedBuy,           // 49
    PumpWrappedSell,          // 50
    PerpsV2,                  // 51
    PerpsV2AddLiquidity,      // 52
    PerpsV2RemoveLiquidity,   // 53
    MoonshotWrappedBuy,       // 54
    MoonshotWrappedSell,      // 55
    StabbleStableSwap,        // 56
    StabbleWeightedSwap,      // 57
    Obric {
        x_to_y: bool,
    }, // 58
    FoxBuyFromEstimatedCost,  // 59
    FoxClaimPartial {
        is_y: bool,
    }, // 60
    SolFi {
        is_quote_to_base: bool,
    }, // 61
    SolayerDelegateNoInit,    // 62
    SolayerUndelegateNoInit,  // 63
    TokenMill {
        side: JupSide,
    }, // 64
    DaosFunBuy,               // 65
    DaosFunSell,              // 66
    ZeroFi,                   // 67
    StakeDexWithdrawWrappedSol, // 68
    VirtualsBuy,              // 69
    VirtualsSell,             // 70
    Perena {
        in_index: u8,
        out_index: u8,
    }, // 71
    PumpSwapBuy,              // 72
    PumpSwapSell,             // 73
    Gamma,                    // 74
    MeteoraDlmmSwapV2 {
        remaining_accounts_info: RemainingAccountsInfo,
    }, // 75
    Woofi,                    // 76
    MeteoraDammV2,            // 77
    MeteoraDynamicBondingCurveSwap, // 78
    StabbleStableSwapV2,      // 79
    StabbleWeightedSwapV2,    // 80
    RaydiumLaunchlabBuy {
        share_fee_rate: u64,
    }, // 81
    RaydiumLaunchlabSell {
        share_fee_rate: u64,
    }, // 82
    BoopdotfunWrappedBuy,     // 83
    BoopdotfunWrappedSell,    // 84
    Plasma {
        side: JupSide,
    }, // 85
    GoonFi {
        is_bid: bool,
        blacklist_bump: u8,
    }, // 86
    HumidiFi {
        swap_id: u64,
        is_base_to_quote: bool,
    }, // 87
    MeteoraDynamicBondingCurveSwapWithRemainingAccounts, // 88
    TesseraV {
        side: JupSide,
    }, // 89
    PumpWrappedBuyV2,         // 90
    PumpWrappedSellV2,        // 91
    PumpSwapBuyV2,            // 92
    PumpSwapSellV2,           // 93
    Heaven {
        a_to_b: bool,
    }, // 94
    SolFiV2 {
        is_quote_to_base: bool,
    }, // 95
    Aquifer,                  // 96
    PumpWrappedBuyV3,         // 97
    PumpWrappedSellV3,        // 98
    PumpSwapBuyV3,            // 99
    PumpSwapSellV3,           // 100
    JupiterLendDeposit,       // 101
    JupiterLendRedeem,        // 102
    DefiTuna {
        a_to_b: bool,
        remaining_accounts_info: Option<RemainingAccountsInfo>,
    }, // 103
    AlphaQ {
        a_to_b: bool,
    }, // 104
    RaydiumV2,                // 105
    SarosDlmm {
        swap_for_y: bool,
    }, // 106
    Futarchy {
        side: JupSide,
    }, // 107
    MeteoraDammV2WithRemainingAccounts, // 108
    Obsidian,                 // 109
    WhaleStreet {
        side: JupSide,
    }, // 110
    DynamicV1 {
        candidate_swaps: Vec<CandidateSwap>,
        best_position: Option<u8>,
    }, // 111
    PumpWrappedBuyV4,         // 112
    PumpWrappedSellV4,        // 113
    CarrotIssue,              // 114
    CarrotRedeem,             // 115
    Manifest {
        side: JupSide,
    }, // 116
    BisonFi {
        a_to_b: bool,
    }, // 117
    HumidiFiV2 {
        swap_id: u64,
        is_base_to_quote: bool,
    }, // 118
    PerenaStar {
        is_mint: bool,
    }, // 119
    JupiterRfqV2 {
        side: JupSide,
        fill_data: Vec<u8>,
    }, // 120
    GoonFiV2 {
        is_bid: bool,
    }, // 121
    Scorch {
        swap_id: u128,
    }, // 122
    VaultLiquidUnstake {
        lst_amounts: [u64; 5],
        seed: u64,
    }, // 123
    XOrca,                    // 124
    Quantum {
        side: JupSide,
    }, // 125
}

#[derive(Debug, Clone, BorshDeserialize)]
#[allow(dead_code)]
struct RoutePlanStep {
    swap: Swap,
    percent: u8,
    input_index: u8,
    output_index: u8,
}

#[derive(Debug, Clone, BorshDeserialize)]
#[allow(dead_code)]
struct RoutePlanStepV2 {
    swap: Swap,
    bps: u16,
    input_index: u8,
    output_index: u8,
}

// The ten route variants share three wire layouts. `deser` ignores trailing
// bytes, and `shared_accounts_*` only prefixes a `u8` id, so one struct per
// layout covers them all. Exact-out variants put an out_amount where exact-in
// puts in_amount — same width, different meaning (see `RouteShape::exact_in`).

/// route / exact_out_route (+ their `shared_accounts_*` forms).
#[derive(BorshDeserialize)]
struct V1Args {
    route_plan: Vec<RoutePlanStep>,
    amount: u64,
    #[allow(dead_code)]
    quoted_amount: u64,
    #[allow(dead_code)]
    slippage_bps: u16,
    platform_fee_bps: u8,
}

/// `*_with_token_ledger`: the amount comes from the ledger, not the args.
#[derive(BorshDeserialize)]
struct V1LedgerArgs {
    route_plan: Vec<RoutePlanStep>,
    #[allow(dead_code)]
    quoted_amount: u64,
    #[allow(dead_code)]
    slippage_bps: u16,
    platform_fee_bps: u8,
}

/// All four v2 variants: fixed prefix, `route_plan` last.
#[derive(BorshDeserialize)]
struct V2Args {
    amount: u64,
    #[allow(dead_code)]
    quoted_amount: u64,
    #[allow(dead_code)]
    slippage_bps: u16,
    platform_fee_bps: u16,
    #[allow(dead_code)]
    positive_slippage_bps: u16,
    route_plan: Vec<RoutePlanStepV2>,
}

/// Which wire layout a route discriminator uses.
#[derive(Clone, Copy)]
struct RouteShape {
    /// `shared_accounts_*` prefixes the args with a `u8` id.
    id_prefix: bool,
    /// v2 layout ([`V2Args`]) rather than v1.
    v2: bool,
    /// `*_with_token_ledger` ([`V1LedgerArgs`]); never set for v2.
    ledger: bool,
    /// The leading amount is the route's *input*. Exact-out variants carry an
    /// out_amount there instead, which says nothing about the input.
    exact_in: bool,
}

fn route_shape(disc: [u8; 8]) -> Option<RouteShape> {
    let (id_prefix, v2, ledger, exact_in) = match disc {
        ROUTE => (false, false, false, true),
        EXACT_OUT_ROUTE => (false, false, false, false),
        ROUTE_WITH_TOKEN_LEDGER => (false, false, true, false),
        SHARED_ACCOUNTS_ROUTE => (true, false, false, true),
        SHARED_ACCOUNTS_EXACT_OUT_ROUTE => (true, false, false, false),
        SHARED_ACCOUNTS_ROUTE_WITH_TOKEN_LEDGER => (true, false, true, false),
        ROUTE_V2 => (false, true, false, true),
        EXACT_OUT_ROUTE_V2 => (false, true, false, false),
        SHARED_ACCOUNTS_ROUTE_V2 => (true, true, false, true),
        SHARED_ACCOUNTS_EXACT_OUT_ROUTE_V2 => (true, true, false, false),
        _ => return None,
    };
    Some(RouteShape {
        id_prefix,
        v2,
        ledger,
        exact_in,
    })
}

/// Split a route instruction into its shape and its Borsh args (past the
/// discriminator and any `shared_accounts_*` id byte).
fn split_route_args(data: &[u8]) -> Option<(RouteShape, &[u8])> {
    let disc: [u8; 8] = data.get(..8)?.try_into().ok()?;
    let shape = route_shape(disc)?;
    Some((shape, data.get(8 + shape.id_prefix as usize..)?))
}

pub fn decode_jupiter_rfq_fill(data: &[u8]) -> Option<(FillExactInInstruction, FillAnalysis)> {
    if let Some((steps, in_amount, _)) = parse_route_steps(data) {
        if let Some(result) = steps
            .iter()
            .find_map(|(side, fill_data, input_idx, _, bps)| {
                let reliable_in = if *input_idx == 0 {
                    in_amount.map(|r| apply_bps(r, *bps))
                } else {
                    None
                };
                try_decode_rfq_fill(side, fill_data, reliable_in)
            })
        {
            return Some(result);
        }
        if !steps.is_empty() {
            return None;
        }
    }
    scan_via_tag_byte_full(data)
}

fn route_in_amount_from_header(data: &[u8]) -> Option<u64> {
    let disc: [u8; 8] = data.get(..8)?.try_into().ok()?;
    let offset = match disc {
        ROUTE_V2 => 8,
        SHARED_ACCOUNTS_ROUTE_V2 => 9,
        _ => return None,
    };
    let bytes: [u8; 8] = data.get(offset..offset + 8)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn apply_bps(route_in: u64, bps: u16) -> u64 {
    ((route_in as u128) * (bps as u128) / 10_000) as u64
}

fn scan_via_tag_byte_full(data: &[u8]) -> Option<(FillExactInInstruction, FillAnalysis)> {
    const JUPITER_RFQ_V2_TAG: u8 = 120;
    let route_in_amount = route_in_amount_from_header(data);

    for i in 0..data.len() {
        if data[i] != JUPITER_RFQ_V2_TAG {
            continue;
        }
        if i + 6 > data.len() {
            break;
        }
        let side = data[i + 1];
        if side > 1 {
            continue;
        }
        let fill_data_len =
            u32::from_le_bytes([data[i + 2], data[i + 3], data[i + 4], data[i + 5]]) as usize;
        if !(16..=1024).contains(&fill_data_len) {
            continue;
        }
        let fill_data_end = i + 6 + fill_data_len;
        if fill_data_end + 4 > data.len() {
            continue;
        }
        let fill_data = &data[i + 6..fill_data_end];
        let jup_side = if side == 0 {
            JupSide::Bid
        } else {
            JupSide::Ask
        };
        let bps = u16::from_le_bytes([data[fill_data_end], data[fill_data_end + 1]]);
        let input_idx = data[fill_data_end + 2];
        let reliable_in = if input_idx == 0 {
            route_in_amount.map(|r| apply_bps(r, bps))
        } else {
            None
        };
        if let Some(result) = try_decode_rfq_fill(&jup_side, fill_data, reliable_in) {
            return Some(result);
        }
    }
    None
}

pub fn extract_platform_fee_bps(data: &[u8]) -> Option<u16> {
    let (shape, args) = split_route_args(data)?;
    Some(if shape.v2 {
        deser::<V2Args>(args)?.platform_fee_bps
    } else if shape.ledger {
        deser::<V1LedgerArgs>(args)?.platform_fee_bps as u16
    } else {
        deser::<V1Args>(args)?.platform_fee_bps as u16
    })
}

fn parse_route_steps(data: &[u8]) -> Option<(Vec<RfqStep>, Option<u64>, u32)> {
    let (shape, args) = split_route_args(data)?;
    let (steps, amount, total) = if shape.v2 {
        let a = deser::<V2Args>(args)?;
        let total = a.route_plan.len() as u32;
        (extract_rfq_steps_v2(&a.route_plan), Some(a.amount), total)
    } else if shape.ledger {
        let a = deser::<V1LedgerArgs>(args)?;
        let total = a.route_plan.len() as u32;
        (extract_rfq_steps_v1(&a.route_plan), None, total)
    } else {
        let a = deser::<V1Args>(args)?;
        let total = a.route_plan.len() as u32;
        (extract_rfq_steps_v1(&a.route_plan), Some(a.amount), total)
    };
    // Only an exact-in route's leading amount is the route input.
    Some((steps, amount.filter(|_| shape.exact_in), total))
}

fn extract_rfq_steps_v1(plan: &[RoutePlanStep]) -> Vec<RfqStep> {
    plan.iter()
        .filter_map(|step| match &step.swap {
            Swap::JupiterRfqV2 { side, fill_data } => Some((
                side.clone(),
                fill_data.clone(),
                step.input_index,
                step.output_index,
                (step.percent as u16).saturating_mul(100),
            )),
            _ => None,
        })
        .collect()
}

fn extract_rfq_steps_v2(plan: &[RoutePlanStepV2]) -> Vec<RfqStep> {
    plan.iter()
        .filter_map(|step| match &step.swap {
            Swap::JupiterRfqV2 { side, fill_data } => Some((
                side.clone(),
                fill_data.clone(),
                step.input_index,
                step.output_index,
                step.bps,
            )),
            _ => None,
        })
        .collect()
}

fn to_rfq_side(side: &JupSide) -> Side {
    match side {
        JupSide::Bid => Side::Bid,
        JupSide::Ask => Side::Ask,
    }
}

fn try_decode_rfq_fill(
    jup_side: &JupSide,
    fill_data: &[u8],
    in_amount: Option<u64>,
) -> Option<(FillExactInInstruction, FillAnalysis)> {
    let side = to_rfq_side(jup_side);

    if let Some(ix) = deser::<FillExactInInstruction>(fill_data) {
        if let Ok(analysis) = analyze_fill(&ix) {
            if analysis.levels_consumed > 0 {
                return Some((ix, analysis));
            }
        }
    }

    if fill_data.len() > 8 {
        if let Some(ix) = deser::<FillExactInInstruction>(&fill_data[8..]) {
            if let Ok(analysis) = analyze_fill(&ix) {
                if analysis.levels_consumed > 0 {
                    return Some((ix, analysis));
                }
            }
        }
    }

    if let Some(params) = deser::<FillExactInParams>(fill_data) {
        if !is_params_plausible(&params) {
            return None;
        }
        // `amount_in_atoms` is 0 when this leg's runtime input isn't
        // derivable from the static route header (i.e. not the first
        // leg). The sweep returns zeros in that case but the params
        // are still surfaced.
        let ix = FillExactInInstruction {
            taker_side: side,
            amount_in_atoms: in_amount.unwrap_or(0),
            params,
        };
        let analysis = analyze_fill(&ix).ok()?;

        if in_amount.is_some() && analysis.levels_consumed == 0 {
            return None;
        }
        return Some((ix, analysis));
    }

    None
}
