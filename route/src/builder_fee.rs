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

    pub fn bps(self, ceiling: BuilderFeeCeilingBps) -> u16 {
        match self {
            Self::Taker => ceiling.taker_bps,
            Self::MakerOrTaker => ceiling.maker_bps.max(ceiling.taker_bps),
        }
    }
}

/// Everything the owner is shown and the order is sized against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeBound {
    pub code_hex: String,
    pub builder_bps: u16,
    pub basis: FeeRateBasis,
    pub notional_micro: u64,
    pub builder_fee_micro: u64,
}

impl FeeBound {
    pub fn total_fee_micro(&self) -> u64 {
        self.builder_fee_micro
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
) -> Result<FeeBound, String> {
    let basis = FeeRateBasis::for_order_type(order_type);
    let builder_bps = basis.bps(ceiling);
    let notional_micro = fee_notional_micro(quote, best_bid_micro)?;
    let builder_fee_micro = builder_fee_ceiling_micro(notional_micro, builder_bps)?;
    Ok(FeeBound {
        code_hex: code_hex.to_owned(),
        builder_bps,
        basis,
        notional_micro,
        builder_fee_micro,
    })
}

/// The ceremony advisory. The owner sees which builder is paid, the rate that
/// bounds it, the total this bound covers, and that Polymarket's own fee sits
/// outside it.
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
        "Polymarket charges its own taker fee on this trade. It is NOT inside the total below and comes on top of it: its published formula has terms this Petal cannot resolve, and bounding it would mean inventing a number."
            .to_owned(),
        format!(
            "Total authorized: {} pUSD, covering the order and the builder fee.",
            format_micro(bound.spend_including_fees_micro())
        ),
        "Polymarket sets a builder's rates on its profile and serves no API for them, so the builder figure is this installation's configured ceiling, not a quote. The venue charges at match time and may charge less."
            .to_owned(),
    ];
    if spend_reduced {
        lines.push(
            "The order was sized down so its notional plus the builder fee fits the amount requested."
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

    /// The bound covers the builder fee only. Polymarket's own fee is stated
    /// as outside it, because its formula has terms this Petal cannot resolve
    /// and an earlier revision that bounded it at the limit price understated
    /// real fills by several times.
    #[test]
    fn the_bound_covers_the_builder_fee_and_says_the_venue_fee_is_outside_it() {
        let bound = fee_bound(
            "0xabc",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
            None,
        )
        .unwrap();
        assert_eq!(bound.builder_fee_micro, 100_000);
        assert_eq!(bound.total_fee_micro(), 100_000);
        assert_eq!(bound.spend_including_fees_micro(), 10_100_000);

        let advisory = String::from_utf8(builder_code_order_advisory(&bound, true)).unwrap();
        assert!(advisory.contains("0xabc"), "{advisory}");
        assert!(advisory.contains("1%"), "{advisory}");
        assert!(advisory.contains("10.1"), "{advisory}");
        // The owner must be told the venue fee is not in the total, not shown
        // a number for it.
        assert!(advisory.contains("NOT inside the total"), "{advisory}");
        assert!(advisory.contains("on top of it"), "{advisory}");
        assert!(advisory.contains("serves no API"), "{advisory}");
        assert!(advisory.contains("sized down"), "{advisory}");
    }
}
