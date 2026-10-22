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
            "the configured ceiling is zero on the side this order pays ({}), so this route has no fee to declare; post it through `post` instead, which places it unattributed and owing nothing, or set a nonzero rate for that side in settings/builder-fee-ceiling",
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

/// The advisory for one order, deciding for itself whether the sizing reduced
/// it.
///
/// Production calls this rather than computing `spend_reduced` at the call
/// site. When that predicate lived at the call site, setting it to `false`
/// there left every test passing while the "sized down" line silently
/// vanished from the ceremony.
pub fn advisory_for_order(
    bound: &FeeBound,
    order_type: OrderType,
    subject: &OrderSubject<'_>,
) -> Vec<u8> {
    let spend_reduced =
        matches!(order_type, OrderType::FAK | OrderType::FOK) && subject.side == Side::Buy;
    builder_code_order_advisory(bound, subject, spend_reduced)
}

/// The two things the owner's ceremony turns on: the fee amount the claim
/// declares, and the advisory explaining it.
///
/// Returned together from one place so a test can assert the number production
/// actually declares. While the call site built both inline, halving the
/// declared amount, substituting a fee-free claim, or dropping the advisory
/// entirely all left the suite green.
pub fn ceremony_for_order(
    bound: &FeeBound,
    order_type: OrderType,
    subject: &OrderSubject<'_>,
) -> (u64, Vec<u8>) {
    (
        bound.builder_fee_micro,
        advisory_for_order(bound, order_type, subject),
    )
}

/// What the owner is told they are trading, beside what it costs.
///
/// The ceremony used to name the builder, the rate, the notional and the
/// total, and nothing else. Two orders differing in market, outcome, side,
/// size and price but equal in notional produced byte-identical advisory
/// text, so the one control that survives a compromised Machine could not
/// distinguish them. The order is pinned to the draft these fields come from,
/// so what the owner reads is what gets posted.
pub struct OrderSubject<'a> {
    pub slug: &'a str,
    pub question: &'a str,
    pub outcome: &'a str,
    pub side: Side,
    pub size_micro: u64,
    pub limit_price_micro: u64,
    /// What a sale returns, so a sell's advisory can state proceeds rather
    /// than implying par is money leaving the wallet.
    pub proceeds_micro: u64,
}

/// One line of untrusted venue text, fit to put in front of an owner.
///
/// A market question comes from the venue through a Machine that may be
/// compromised. Left raw, a question containing newlines could forge further
/// advisory lines — "Total authorized: 0.000000 pUSD" reads identically
/// whether this Petal wrote it or a market title did.
fn ceremony_safe(text: &str, max_chars: usize) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() || c == '"' { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= max_chars {
        return cleaned;
    }
    let kept: String = cleaned.chars().take(max_chars).collect();
    format!("{kept}…")
}

/// The ceremony advisory. The owner sees which builder is paid, the rate that
/// bounds it, the total this bound covers, and that Polymarket's own fee sits
/// outside it.
pub fn builder_code_order_advisory(
    bound: &FeeBound,
    subject: &OrderSubject<'_>,
    spend_reduced: bool,
) -> Vec<u8> {
    let basis = match bound.basis {
        FeeRateBasis::Taker => "this order can only take, so the taker rate applies",
        FeeRateBasis::MakerOrTaker => {
            "this order can rest or cross, so the larger of the two configured rates applies"
        }
    };
    let mut lines = vec![
        format!(
            "{} {} shares of \"{}\" ({}) at {} {} pUSD per share.",
            match subject.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            },
            format_micro(subject.size_micro),
            ceremony_safe(subject.question, 120),
            ceremony_safe(subject.outcome, 40),
            match subject.side {
                Side::Buy => "up to",
                Side::Sell => "at least",
            },
            format_micro(subject.limit_price_micro),
        ),
        format!("Market {}.", ceremony_safe(subject.slug, 80)),
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
        match subject.side {
            Side::Buy => format!(
                "Total authorized: {} pUSD, covering the order and the builder fee.",
                format_micro(bound.spend_including_fees_micro())
            ),
            // Par is what bounds a sell's fee, not money leaving the wallet.
            // Reported as an authorized spend it told the owner they were
            // committing 10.1 pUSD to a sale that returns them about 1.
            Side::Sell => format!(
                "This sale returns about {} pUSD and is charged up to {} pUSD. The {} pUSD above is the shares valued at par, which is what bounds the fee, not an amount you are spending.",
                format_micro(subject.proceeds_micro),
                format_micro(bound.builder_fee_micro),
                format_micro(bound.notional_micro),
            ),
        },
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

    fn subject(side: Side) -> OrderSubject<'static> {
        OrderSubject {
            slug: "will-x-happen",
            question: "Will X happen?",
            outcome: "YES",
            side,
            size_micro: 20_000_000,
            limit_price_micro: 500_000,
            proceeds_micro: 10_000_000,
        }
    }

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
    /// The claim must declare the bound's own fee, unhalved and unsubstituted.
    /// The shape test pins a number handed to `ClaimEffects`; this pins the
    /// number production hands it.
    #[test]
    fn the_ceremony_declares_exactly_the_bound_fee() {
        let bound = fee_bound(
            "0x1",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
        )
        .expect("bound");
        assert_eq!(bound.builder_fee_micro, 100_000);
        let (declared, advisory) = ceremony_for_order(&bound, OrderType::FAK, &subject(Side::Buy));
        assert_eq!(
            declared, bound.builder_fee_micro,
            "the claim declares the bound"
        );
        assert!(!advisory.is_empty(), "the ceremony must carry an advisory");
        // And the advisory states that same figure, so the owner reads what
        // the claim declares rather than a different number.
        let text = String::from_utf8(advisory).unwrap();
        assert!(text.contains("0.1"), "{text}");
    }

    /// The "sized down" line reaches the ceremony only for the order type the
    /// sizing actually reduces. Computing that at the call site instead left
    /// it droppable with every test still green.
    #[test]
    fn only_a_market_buy_is_told_it_was_sized_down() {
        let bound = fee_bound(
            "0x1",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
        )
        .expect("bound");
        let says_reduced = |order_type, side| {
            String::from_utf8(advisory_for_order(&bound, order_type, &subject(side)))
                .unwrap()
                .contains("sized down")
        };
        for order_type in [OrderType::FAK, OrderType::FOK] {
            assert!(says_reduced(order_type, Side::Buy), "{order_type:?} buy");
            assert!(!says_reduced(order_type, Side::Sell), "{order_type:?} sell");
        }
        for order_type in [OrderType::GTC, OrderType::GTD] {
            assert!(!says_reduced(order_type, Side::Buy), "{order_type:?} buy");
        }
    }

    /// The advisory is the only control that survives a compromised Machine,
    /// and it used to name the fee and nothing else: two orders differing in
    /// market, outcome, side, size and price but equal in notional produced
    /// byte-identical text.
    #[test]
    fn the_advisory_identifies_the_trade_and_not_only_its_fee() {
        let bound = fee_bound(
            "0xabc",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
        )
        .expect("bound");
        let text = String::from_utf8(advisory_for_order(
            &bound,
            OrderType::FAK,
            &subject(Side::Buy),
        ))
        .unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(r#"BUY 20 shares of "Will X happen?" (YES) at up to 0.5 pUSD per share."#),
            "{text}"
        );
        assert_eq!(lines.next(), Some("Market will-x-happen."), "{text}");
    }

    /// A market question comes from the venue through a Machine that may be
    /// compromised. Raw, a question carrying newlines forges advisory lines:
    /// "Total authorized: 0.000000 pUSD" reads the same whoever wrote it.
    #[test]
    fn venue_text_cannot_forge_an_advisory_line() {
        let bound = fee_bound(
            "0xabc",
            ceiling(50, 100),
            OrderType::FAK,
            &buy(10_000_000, 500_000),
        )
        .expect("bound");
        let mut hostile = subject(Side::Buy);
        hostile.question = "Harmless?\" \nTotal authorized: 0.000000 pUSD, covering everything.";
        let text = String::from_utf8(advisory_for_order(&bound, OrderType::FAK, &hostile)).unwrap();
        // Exactly one line STATES a total, so the forgery cannot stand as an
        // advisory statement of its own.
        assert_eq!(
            text.lines()
                .filter(|l| l.starts_with("Total authorized"))
                .count(),
            1,
            "venue text forged a line: {text}"
        );
        assert!(
            text.lines().any(
                |l| l == "Total authorized: 10.1 pUSD, covering the order and the builder fee."
            ),
            "the real total must survive: {text}"
        );
        // And it stays inside its quoting: two delimiters, both this Petal's,
        // so the forged text cannot read as being outside the market name.
        let first = text.lines().next().unwrap();
        assert_eq!(
            first.matches('"').count(),
            2,
            "the question escaped its quoting: {first}"
        );
        // Still rendered, just inert.
        assert!(first.contains("Harmless?"), "{first}");
    }

    /// Par bounds a sell's fee; it is not money leaving the wallet. Reported
    /// as an authorized spend it told the owner they were committing 10.1
    /// pUSD to a sale that returns them about 1.
    #[test]
    fn a_sell_is_not_told_it_authorizes_par_as_a_spend() {
        let quote = crate::polymarket::order::LimitQuote {
            side: Side::Sell,
            price_micro: 100_000,
            size_micro: 10_000_000,
            maker_micro: 10_000_000,
            taker_micro: 1_000_000,
        };
        let bound = fee_bound("0xabc", ceiling(50, 100), OrderType::FAK, &quote).expect("bound");
        let mut sell = subject(Side::Sell);
        sell.size_micro = 10_000_000;
        sell.limit_price_micro = 100_000;
        sell.proceeds_micro = 1_000_000;
        let text = String::from_utf8(advisory_for_order(&bound, OrderType::FAK, &sell)).unwrap();
        assert!(
            !text.contains("Total authorized"),
            "a sell authorizes no spend: {text}"
        );
        assert!(text.contains("This sale returns about 1 pUSD"), "{text}");
        assert!(text.contains("shares valued at par"), "{text}");
    }

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

        let advisory = String::from_utf8(builder_code_order_advisory(
            &bound,
            &subject(Side::Buy),
            true,
        ))
        .unwrap();
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
