//! Bounding what a builder-coded order can cost the owner.
//!
//! Polymarket charges two fees and neither is in the signed order: both are
//! priced by its services at match time, so nothing the owner signs commits
//! the venue to an amount. What the owner can be given is a **ceiling** on the
//! one fee this Petal can compute, and `makerAmount` — which *is* signed —
//! sized so the order plus that ceiling fits the spend they approved.
//!
//! - **Builder fee**: `notional * rate_bps / 10_000`, flat. Fully specified by
//!   Polymarket's documentation, with a worked example confirming notional
//!   means pUSD. No API serves a builder profile's registered rates, so the
//!   rate is the operator's declared ceiling (see
//!   [`crate::builder_code::BuilderFeeCeilingBps`]). This is the leg declared
//!   in the claim and sized against.
//! - **Platform fee**: not computed here at all. Three of its inputs are
//!   unresolved: the documentation defines its first term only as "the trade
//!   size" and never links it to notional the way the builder section does,
//!   giving no worked example to disambiguate shares from pUSD; the exponent's
//!   meaning is unverified; and its `p * (1 - p)` factor peaks at $0.50 rather
//!   than rising with price, so pricing it at an order's limit understated
//!   real fills by 3-5x in an earlier revision. The owner is told it is
//!   charged separately and sits outside the bound, which is true where a
//!   number would not be.
use crate::builder_code::{BuilderFeeCeilingBps, bps_percent_string, builder_fee_ceiling_micro};
use crate::polymarket::Side;
use crate::polymarket::order::{LimitQuote, OrderType, format_micro};

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
/// buy exactly: a taker buy that fills better spends less.
///
/// A sell is bounded at par — `size_micro`, the shares valued at $1 each —
/// rather than at its limit or at the book. An outcome share cannot fill above
/// $1, so par can never understate. The alternatives both fail: the limit
/// understates whenever a taker sell matches at the maker's better price, and
/// the best bid moves, which would make the declared fee drift between the
/// draft, the revalidation and the post. The fee is compared across those
/// stages, so a bid-dependent bound would refuse a builder-coded sell on any
/// tick of the book while its maker and taker legs sat still. Par costs
/// tightness — at a $0.10 fill it over-declares tenfold — but the rate is at
/// most 1%, so the absolute over-statement is at most 1% of par, and the owner
/// sees the figure before approving it.
pub fn fee_notional_micro(quote: &LimitQuote) -> u64 {
    match quote.side {
        Side::Buy => quote.maker_micro,
        Side::Sell => quote.size_micro,
    }
}

/// The ceiling for one order.
pub fn fee_bound(
    code_hex: &str,
    ceiling: BuilderFeeCeilingBps,
    order_type: OrderType,
    quote: &LimitQuote,
) -> Result<FeeBound, String> {
    let basis = FeeRateBasis::for_order_type(order_type);
    let builder_bps = basis.bps(ceiling);
    // A ceiling may legitimately be zero on one side, but the side this order
    // can actually be charged on must not be: the fee-bearing class cannot
    // carry a zero fee, and the retry path refuses such an artifact outright,
    // so preparing one would sign something no retry could ever resume.
    if builder_bps == 0 {
        return Err(format!(
            "the configured ceiling is zero on the side this order pays ({}), so it would declare no fee; set a nonzero rate for that side in settings/builder-fee-ceiling, or clear settings/builder-code to trade without attribution",
            match basis {
                FeeRateBasis::Taker => "taker",
                FeeRateBasis::MakerOrTaker => "maker and taker",
            }
        ));
    }
    let notional_micro = fee_notional_micro(quote);
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
        match bound.basis {
            FeeRateBasis::Taker => "Polymarket charges its own taker fee on this trade. It is NOT inside the total below and comes on top of it: its published formula has terms this Petal cannot resolve, and bounding it would mean inventing a number.",
            FeeRateBasis::MakerOrTaker => "If this order takes rather than rests, Polymarket charges its own taker fee as well. It is NOT inside the total below and would come on top of it: its published formula has terms this Petal cannot resolve, and bounding it would mean inventing a number.",
        }
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

    /// A buy is bounded by what it gives. The fixture uses different maker
    /// and taker legs on purpose: equal ones cannot tell the two apart, so a
    /// regression to `taker_micro` would pass unnoticed.
    #[test]
    fn a_buy_is_bounded_by_what_the_maker_gives() {
        let quote = LimitQuote {
            side: Side::Buy,
            price_micro: 500_000,
            size_micro: 20_000_000,
            maker_micro: 10_000_000,
            taker_micro: 20_000_000,
        };
        assert_eq!(fee_notional_micro(&quote), 10_000_000);
    }

    /// A sell is bounded at par, independent of its limit and of the book. A
    /// share cannot fill above $1, so par never understates, and because it
    /// does not move the declared fee cannot drift between the stages that
    /// compare it.
    #[test]
    fn a_sell_is_bounded_at_par_regardless_of_limit_or_book() {
        let cheap = sell(1_000_000, 10_000_000, 100_000);
        let dear = sell(9_000_000, 10_000_000, 900_000);
        // Same shares, wildly different limits: same bound.
        assert_eq!(fee_notional_micro(&cheap), 10_000_000);
        assert_eq!(fee_notional_micro(&dear), 10_000_000);
        // And it is never below what the limit alone implies.
        assert!(fee_notional_micro(&dear) >= dear.taker_micro);
    }

    #[test]
    fn an_immediate_order_uses_the_taker_rate_and_a_resting_one_the_larger() {
        for order_type in [OrderType::FAK, OrderType::FOK] {
            assert_eq!(
                FeeRateBasis::for_order_type(order_type).bps(ceiling(25, 100)),
                100
            );
        }
        // Two pairs, so the max is pinned against BOTH fields: an inverted
        // pair rules out reading the taker field, and a taker-dominant pair
        // rules out reading the maker field. One pair alone cannot.
        for order_type in [OrderType::GTC, OrderType::GTD] {
            assert_eq!(
                FeeRateBasis::for_order_type(order_type).bps(ceiling(50, 10)),
                50
            );
            assert_eq!(
                FeeRateBasis::for_order_type(order_type).bps(ceiling(10, 100)),
                100
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

    /// A ceiling may be zero on one side, but not on the side this order
    /// pays: the fee-bearing class cannot carry a zero fee, and the retry path
    /// refuses such an artifact, so preparing one would sign something no
    /// retry could resume.
    #[test]
    fn a_ceiling_of_zero_on_the_paying_side_is_refused_at_prepare_time() {
        let maker_only = ceiling(50, 0);
        // A market order can only take, so it pays the zero side.
        for order_type in [OrderType::FAK, OrderType::FOK] {
            assert!(
                fee_bound("0x1", maker_only, order_type, &buy(10_000_000, 500_000)).is_err(),
                "{order_type:?} pays the taker rate, which is zero here"
            );
        }
        // A resting order pays max(50, 0), which is nonzero, so it is fine.
        for order_type in [OrderType::GTC, OrderType::GTD] {
            let bound = fee_bound("0x1", maker_only, order_type, &buy(10_000_000, 500_000))
                .expect("a resting order pays the larger rate");
            assert_eq!(bound.builder_bps, 50);
        }
        // A taker-only ceiling is the mirror image.
        assert!(
            fee_bound(
                "0x1",
                ceiling(0, 100),
                OrderType::FAK,
                &buy(10_000_000, 500_000)
            )
            .is_ok()
        );
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
