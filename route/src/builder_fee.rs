//! Bounding what a builder-coded order can cost the owner.
//!
//! Neither fee Polymarket charges is in the signed order. The Fees Service
//! prices both at match time and the Builders Service accrues the builder
//! share from indexed `OrderFilled` events, so nothing the owner signs commits
//! the venue to an amount. What the owner can be given instead is a **ceiling**
//! that never understates, computed from the configured builder rate and the
//! market's published platform rate, and `makerAmount` — which *is* signed —
//! sized so the order plus those fees fits the spend they approved.
//!
//! Two layers, with different availability:
//!
//! - **Builder fee**: `notional * rate_bps / 10_000`, flat and additive. No API
//!   serves a builder profile's registered rates, so the rate is the operator's
//!   declared ceiling (see [`crate::builder_code::BuilderFeeCeilingBps`]).
//! - **Platform fee**: `C * feeRate * p * (1 - p)`, taker only, per market, read
//!   from `getClobMarketInfo`.
use crate::builder_code::{BuilderFeeCeilingBps, bps_percent_string, builder_fee_ceiling_micro};
use crate::polymarket::Side;
use crate::polymarket::order::{LimitQuote, MICRO, OrderType, format_micro};

/// Which side's rate a builder fee is bounded by, and why.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeeRateBasis {
    /// Fill-or-kill and fill-and-kill can only take.
    Taker,
    /// A resting order can cross on placement and then rest, so a single
    /// order can mix maker and taker fills. The larger rate bounds both.
    MakerOrTaker,
}

impl FeeRateBasis {
    pub fn for_order_type(order_type: OrderType) -> Self {
        if order_type.can_rest() {
            Self::MakerOrTaker
        } else {
            Self::Taker
        }
    }

    fn bps(self, ceiling: BuilderFeeCeilingBps) -> u16 {
        match self {
            Self::Taker => ceiling.taker_bps,
            Self::MakerOrTaker => ceiling.maker_bps.max(ceiling.taker_bps),
        }
    }

    /// Only a taker pays Polymarket's platform fee. A resting order may still
    /// cross on placement, so it is bounded as if it takes.
    fn pays_platform_fee(self) -> bool {
        true
    }
}

/// The published platform fee parameters for one market, from
/// `getClobMarketInfo`'s `fd` object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformFeeParams {
    /// `fd.r`: the rate numerator.
    pub rate: u64,
    /// `fd.e`: the decimal exponent the rate is scaled by.
    pub exponent: u32,
    /// `fd.to`: charged to takers only.
    pub taker_only: bool,
}

impl PlatformFeeParams {
    /// `ceil(C * rate / 10^e * p * (1 - p))` in micro-pUSD, with `p` in micros.
    ///
    /// The exponent's meaning is taken as a decimal scale on the rate, so an
    /// effective rate of `rate / 10^exponent`. That reading is not confirmed
    /// against a live fee-enabled market, and getting it wrong scales the fee
    /// by a power of ten, which is why `exponent` is carried explicitly rather
    /// than folded into a constant: one place to correct.
    pub fn ceiling_micro(self, notional_micro: u64, price_micro: u64) -> Result<u64, String> {
        if price_micro > MICRO {
            return Err("order price exceeds $1 per share".to_owned());
        }
        let scale = 10_u128
            .checked_pow(self.exponent)
            .ok_or("platform fee exponent is out of range")?;
        // p * (1 - p) in micros^2, then divided back out by MICRO^2.
        let p = u128::from(price_micro);
        let complement = u128::from(MICRO) - p;
        let numerator = u128::from(notional_micro) * u128::from(self.rate) * p * complement;
        let denominator = scale * u128::from(MICRO) * u128::from(MICRO);
        u64::try_from(numerator.div_ceil(denominator))
            .map_err(|_| "platform fee ceiling exceeds the supported pUSD amount".to_owned())
    }
}

/// `getClobMarketInfo`'s fee details. CLOB V2 returns these per market
/// alongside the minimum tick size and order size.
#[derive(Debug, serde::Deserialize)]
pub struct ClobFeeDetails {
    /// `r`: the rate numerator.
    pub r: u64,
    /// `e`: the decimal exponent the rate is scaled by.
    pub e: u32,
    /// `to`: charged to takers only.
    #[serde(default)]
    pub to: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct ClobMarketInfo {
    pub fd: Option<ClobFeeDetails>,
}

impl ClobMarketInfo {
    pub fn platform_fee_params(&self) -> Option<PlatformFeeParams> {
        self.fd.as_ref().map(|fd| PlatformFeeParams {
            rate: fd.r,
            exponent: fd.e,
            taker_only: fd.to,
        })
    }
}

/// Reads one market's published platform fee parameters.
///
/// A failure here is **not** fatal to posting. Platform fees already go
/// unaccounted on plain orders, so refusing to post on a transient read would
/// be a new way to fail rather than a fix. The caller treats `None` as "the
/// platform fee is not bounded in this approval" and the advisory says so, so
/// the owner is told what the number does and does not include.
pub fn read_platform_fee_params(condition_id: &str) -> Option<PlatformFeeParams> {
    let url = format!(
        "{}/clob-market-info?condition_id={condition_id}",
        crate::runtime_config::clob_url()
    );
    crate::infra_parts::http::get_json::<ClobMarketInfo>(&url)
        .ok()
        .and_then(|info| info.platform_fee_params())
}

/// Everything the owner is shown and the order is sized against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeBound {
    pub code_hex: String,
    pub builder_bps: u16,
    pub basis: FeeRateBasis,
    pub notional_micro: u64,
    pub builder_fee_micro: u64,
    /// `None` when the market's fee parameters could not be read, which is
    /// different from a market that charges nothing.
    pub platform_fee_micro: Option<u64>,
}

impl FeeBound {
    pub fn total_fee_micro(&self) -> u64 {
        self.builder_fee_micro
            .saturating_add(self.platform_fee_micro.unwrap_or(0))
    }

    pub fn spend_including_fees_micro(&self) -> u64 {
        self.notional_micro.saturating_add(self.total_fee_micro())
    }
}

/// The pUSD notional a fee is charged on.
///
/// A buy's `maker_micro` is the most pUSD the maker can give, so it bounds the
/// buy exactly. A sell is different: `taker_micro` is derived from the limit,
/// but a taker sell can match at the maker's better price, so realized pUSD can
/// exceed it. Bound a sell by the better of its limit and what the book would
/// pay, clamped at $1 per share because an outcome share cannot fill above par.
/// A builder-coded sell with no bid has no bound at all and is refused.
pub fn fee_notional_micro(quote: &LimitQuote, best_bid_micro: Option<u64>) -> Result<u64, String> {
    match quote.side {
        Side::Buy => Ok(quote.maker_micro),
        Side::Sell => {
            let bid = best_bid_micro.ok_or(
                "a builder-coded sell needs a best bid to bound its fee: the book is empty, so what it would fill at is unknown",
            )?;
            let bid = bid.min(MICRO);
            let at_bid = u64::try_from(
                (u128::from(quote.size_micro) * u128::from(bid)).div_ceil(u128::from(MICRO)),
            )
            .map_err(|_| "sell notional exceeds the supported pUSD amount".to_owned())?;
            Ok(quote.taker_micro.max(at_bid))
        }
    }
}

/// The ceiling for one order.
pub fn fee_bound(
    code_hex: &str,
    ceiling: BuilderFeeCeilingBps,
    order_type: OrderType,
    quote: &LimitQuote,
    best_bid_micro: Option<u64>,
    platform: Option<PlatformFeeParams>,
) -> Result<FeeBound, String> {
    let basis = FeeRateBasis::for_order_type(order_type);
    let builder_bps = basis.bps(ceiling);
    let notional_micro = fee_notional_micro(quote, best_bid_micro)?;
    let builder_fee_micro = builder_fee_ceiling_micro(notional_micro, builder_bps)?;
    let platform_fee_micro = match platform {
        Some(params) if !params.taker_only || basis.pays_platform_fee() => {
            Some(params.ceiling_micro(notional_micro, quote.price_micro)?)
        }
        Some(_) => Some(0),
        None => None,
    };
    Ok(FeeBound {
        code_hex: code_hex.to_owned(),
        builder_bps,
        basis,
        notional_micro,
        builder_fee_micro,
        platform_fee_micro,
    })
}

/// The ceremony advisory. The owner sees which builder is paid, the rate that
/// bounds it, Polymarket's own fee, and the total they are authorizing.
pub fn builder_code_order_advisory(bound: &FeeBound, spend_reduced: bool) -> Vec<u8> {
    let basis = match bound.basis {
        FeeRateBasis::Taker => "this order can only take, so the taker rate applies",
        FeeRateBasis::MakerOrTaker => {
            "this order can rest or cross, so the larger of the two configured rates applies"
        }
    };
    let mut lines = vec![
        format!("Builder code {} is paid on this order.", bound.code_hex),
        format!(
            "Builder fee up to {} pUSD at {} of {} pUSD notional; {basis}.",
            format_micro(bound.builder_fee_micro),
            bps_percent_string(bound.builder_bps),
            format_micro(bound.notional_micro),
        ),
        match bound.platform_fee_micro {
            Some(micro) => format!("Polymarket's own fee up to {} pUSD.", format_micro(micro)),
            None => "Polymarket's own fee could not be read for this market, so it is NOT inside the total below and will be charged on top."
                .to_owned(),
        },
        format!(
            "Total authorized: {} pUSD.",
            format_micro(bound.spend_including_fees_micro())
        ),
        "Polymarket sets a builder's rates on its profile and serves no API for them, so the builder figure is this installation's configured ceiling, not a quote. Both fees are charged by the venue at match time and may be less."
            .to_owned(),
    ];
    if spend_reduced {
        lines.push(
            "The order was sized down so its notional plus these fees fits the amount requested."
                .to_owned(),
        );
    }
    lines.join("\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ceiling(maker_bps: u16, taker_bps: u16) -> BuilderFeeCeilingBps {
        BuilderFeeCeilingBps {
            maker_bps,
            taker_bps,
        }
    }

    fn buy(maker_micro: u64, price_micro: u64) -> LimitQuote {
        LimitQuote {
            side: Side::Buy,
            price_micro,
            size_micro: 10_000_000,
            maker_micro,
            taker_micro: 10_000_000,
        }
    }

    fn sell(taker_micro: u64, size_micro: u64, price_micro: u64) -> LimitQuote {
        LimitQuote {
            side: Side::Sell,
            price_micro,
            size_micro,
            maker_micro: size_micro,
            taker_micro,
        }
    }

    #[test]
    fn a_buy_is_bounded_by_what_the_maker_gives() {
        assert_eq!(
            fee_notional_micro(&buy(10_000_000, 500_000), None),
            Ok(10_000_000)
        );
    }

    #[test]
    fn a_sell_takes_the_larger_of_its_limit_and_the_book() {
        // Limit implies 5 pUSD; the bid would pay 9 pUSD. Take the bid.
        let quote = sell(5_000_000, 10_000_000, 500_000);
        assert_eq!(fee_notional_micro(&quote, Some(900_000)), Ok(9_000_000));
        // A bid below the limit cannot lower the bound.
        assert_eq!(fee_notional_micro(&quote, Some(100_000)), Ok(5_000_000));
    }

    #[test]
    fn a_sell_bid_above_par_is_clamped_at_one_dollar_a_share() {
        let quote = sell(1_000_000, 10_000_000, 100_000);
        assert_eq!(fee_notional_micro(&quote, Some(5_000_000)), Ok(10_000_000));
    }

    #[test]
    fn a_builder_coded_sell_with_no_bid_is_refused() {
        let quote = sell(5_000_000, 10_000_000, 500_000);
        assert!(fee_notional_micro(&quote, None).is_err());
    }

    #[test]
    fn an_immediate_order_uses_the_taker_rate_and_a_resting_one_the_larger() {
        for order_type in [OrderType::FAK, OrderType::FOK] {
            assert_eq!(
                FeeRateBasis::for_order_type(order_type).bps(ceiling(25, 100)),
                100
            );
        }
        // Inverted pair proves the max is taken rather than the taker field.
        for order_type in [OrderType::GTC, OrderType::GTD] {
            assert_eq!(
                FeeRateBasis::for_order_type(order_type).bps(ceiling(50, 10)),
                50
            );
        }
    }

    #[test]
    fn the_builder_fee_rounds_up_where_truncation_would_differ() {
        // 1.000001 pUSD at 1 bp is 0.000100_0001, which must not truncate.
        assert_eq!(builder_fee_ceiling_micro(1_000_001, 1), Ok(101));
        assert_eq!(builder_fee_ceiling_micro(0, 100), Ok(0));
        assert_eq!(builder_fee_ceiling_micro(10_000_000, 100), Ok(100_000));
    }

    #[test]
    fn a_platform_fee_rate_of_zero_costs_nothing_and_par_prices_cost_nothing() {
        let params = PlatformFeeParams {
            rate: 0,
            exponent: 4,
            taker_only: true,
        };
        assert_eq!(params.ceiling_micro(10_000_000, 500_000), Ok(0));
        // p * (1 - p) is zero at both ends of the range.
        let params = PlatformFeeParams {
            rate: 300,
            exponent: 4,
            taker_only: true,
        };
        assert_eq!(params.ceiling_micro(10_000_000, MICRO), Ok(0));
        assert_eq!(params.ceiling_micro(10_000_000, 0), Ok(0));
    }

    #[test]
    fn the_platform_fee_follows_its_published_formula() {
        // 10 pUSD at p = 0.5, rate 300 / 10^4 = 0.03: 10 * 0.03 * 0.25 = 0.075
        let params = PlatformFeeParams {
            rate: 300,
            exponent: 4,
            taker_only: true,
        };
        assert_eq!(params.ceiling_micro(10_000_000, 500_000), Ok(75_000));
        // The exponent is a decimal scale, so raising it by one divides by ten.
        let shifted = PlatformFeeParams {
            rate: 300,
            exponent: 5,
            taker_only: true,
        };
        assert_eq!(shifted.ceiling_micro(10_000_000, 500_000), Ok(7_500));
    }

    #[test]
    fn a_price_above_par_is_refused_rather_than_priced() {
        let params = PlatformFeeParams {
            rate: 300,
            exponent: 4,
            taker_only: true,
        };
        assert!(params.ceiling_micro(10_000_000, MICRO + 1).is_err());
    }

    #[test]
    fn the_bound_sums_both_fees_and_the_advisory_states_each() {
        let params = PlatformFeeParams {
            rate: 300,
            exponent: 4,
            taker_only: true,
        };
        let bound = fee_bound(
            "0xabc",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
            None,
            Some(params),
        )
        .unwrap();
        assert_eq!(bound.builder_fee_micro, 100_000);
        assert_eq!(bound.platform_fee_micro, Some(75_000));
        assert_eq!(bound.total_fee_micro(), 175_000);
        assert_eq!(bound.spend_including_fees_micro(), 10_175_000);

        let advisory = String::from_utf8(builder_code_order_advisory(&bound, true)).unwrap();
        assert!(advisory.contains("0xabc"), "{advisory}");
        assert!(advisory.contains("1%"), "{advisory}");
        assert!(advisory.contains("0.1"), "{advisory}");
        assert!(advisory.contains("10.175"), "{advisory}");
        assert!(advisory.contains("serves no API"), "{advisory}");
        assert!(advisory.contains("sized down"), "{advisory}");
    }

    /// A market whose fee parameters could not be read is not a market that
    /// charges nothing. The owner must be told the total excludes it rather
    /// than shown a zero they would read as "free".
    #[test]
    fn an_unread_platform_fee_is_disclosed_not_shown_as_zero() {
        let bound = fee_bound(
            "0xabc",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
            None,
            None,
        )
        .unwrap();
        let advisory = String::from_utf8(builder_code_order_advisory(&bound, false)).unwrap();
        assert!(advisory.contains("could not be read"), "{advisory}");
        assert!(advisory.contains("charged on top"), "{advisory}");
        assert!(!advisory.contains("own fee up to 0"), "{advisory}");

        // A market that genuinely charges nothing says so as a number.
        let free = fee_bound(
            "0xabc",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
            None,
            Some(PlatformFeeParams {
                rate: 0,
                exponent: 4,
                taker_only: true,
            }),
        )
        .unwrap();
        assert_eq!(free.platform_fee_micro, Some(0));
        let advisory = String::from_utf8(builder_code_order_advisory(&free, false)).unwrap();
        assert!(advisory.contains("own fee up to 0 pUSD"), "{advisory}");
    }

    #[test]
    fn with_no_platform_params_only_the_builder_fee_is_bounded() {
        let bound = fee_bound(
            "0xabc",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
            None,
            None,
        )
        .unwrap();
        assert_eq!(bound.platform_fee_micro, None);
        assert_eq!(bound.total_fee_micro(), 100_000);
    }
}
