//! Shared trade primitives: the live market snapshot + quote building used by
//! both the CLI (`bloom polymarket order/confirm/sell`) and the VFS trade
//! surface. These two callers previously carried byte-identical copies that
//! differed only in error type; this is the single source of truth. It returns
//! [`PolymarketError`]; each caller maps it (anyhow `?` / VFS `err_be`).

#[cfg(feature = "native-client")]
use crate::clob::ClobClient;
#[cfg(feature = "native-client")]
use crate::gamma::GammaClient;
use crate::polymarket::order::{self, LimitQuote, OrderType};
use crate::polymarket::order_model::{DraftStatus, OrderDraft};
#[cfg(feature = "native-client")]
use crate::polymarket::types::BookLevel;
use crate::polymarket::types::{Market, Side};
use crate::polymarket::{PolymarketError, Result};

/// Live market view used at both draft and confirm time.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub market: Market,
    pub token_id: String,
    pub neg_risk: bool,
    pub tick_micro: u64,
    pub min_size_micro: u64,
    pub best_ask_micro: Option<u64>,
    pub best_bid_micro: Option<u64>,
    /// `(price, size)` levels in micro units, best first.
    pub asks: Vec<(u64, u64)>,
    pub bids: Vec<(u64, u64)>,
}

/// A book side as `(price, size)` micro levels, best first: asks ascending,
/// bids descending. The live CLOB lists both best-last, so order is not trusted.
pub fn book_levels(
    levels: &[crate::polymarket::types::BookLevel],
    ask: bool,
) -> Result<Vec<(u64, u64)>> {
    let mut parsed = levels
        .iter()
        .map(|l| Ok((order::parse_micro(&l.price)?, order::parse_micro(&l.size)?)))
        .collect::<Result<Vec<_>>>()?;
    parsed.sort_by_key(|&(price, _)| if ask { price } else { u64::MAX - price });
    Ok(parsed)
}

/// The worst price a marketable order reaches to fill `amount_micro` (USD for a
/// buy, shares for a sell) from `levels`, never past `bound_micro`. As
/// Polymarket's own clients price a market order: the best price alone fills
/// only the top level. `None` when no level is within the bound.
fn fill_price(
    side: Side,
    amount_micro: u64,
    levels: &[(u64, u64)],
    bound_micro: u64,
) -> Option<u64> {
    let mut filled: u128 = 0;
    let mut reached = None;
    for &(price, size) in levels {
        if (side == Side::Buy && price > bound_micro) || (side == Side::Sell && price < bound_micro)
        {
            break;
        }
        reached = Some(price);
        filled += match side {
            Side::Buy => u128::from(price) * u128::from(size) / 1_000_000,
            Side::Sell => u128::from(size),
        };
        if filled >= u128::from(amount_micro) {
            break;
        }
    }
    reached
}

/// Fetch and cross-check the live market for `slug`/`outcome`. Refuses anything
/// that must not be signed against: an incompatible CLOB, a non-binary market, an
/// unknown outcome, or a CLOB↔Gamma disagreement on neg-risk / condition id.
#[cfg(feature = "native-client")]
pub async fn snapshot(
    gamma: &GammaClient,
    clob: &ClobClient,
    slug: &str,
    outcome: &str,
) -> Result<Snapshot> {
    let version = clob.server_version().await?;
    if version != 2 {
        return Err(PolymarketError::invalid(format!(
            "CLOB reports unsupported order version {version}"
        )));
    }

    let market = gamma.market_by_slug(slug).await?;
    // Hard market invariant: bloom only trades true binary YES/NO markets.
    // `yes_token_id`/`no_token_id` index by position, so a multi-outcome market
    // would otherwise be silently traded as binary (and redeem's indexSets=[1,2]
    // is only valid for binary). Refuse before resolving a token.
    if !market.is_binary() {
        return Err(PolymarketError::invalid(format!(
            "market '{slug}' is not a binary YES/NO market (outcomes={:?}); \
             bloom only trades binary markets",
            market.outcomes
        )));
    }
    let token_id = match outcome.to_ascii_uppercase().as_str() {
        "YES" => market.yes_token_id(),
        "NO" => market.no_token_id(),
        other => {
            return Err(PolymarketError::invalid(format!(
                "outcome must be YES or NO, got {other}"
            )));
        }
    }
    .ok_or_else(|| {
        PolymarketError::invalid(format!(
            "market '{slug}' has no token for outcome {outcome}"
        ))
    })?
    .to_string();

    // Data-integrity cross-checks (hard failures, not policy): the CLOB is
    // authoritative for neg-risk and token→condition mapping and must agree
    // with Gamma before anything is signed.
    let neg_risk = clob.neg_risk(&token_id).await?;
    if neg_risk != market.neg_risk {
        return Err(PolymarketError::invalid(format!(
            "neg_risk mismatch for '{slug}': CLOB={neg_risk} Gamma={} — refusing",
            market.neg_risk
        )));
    }
    let tm = clob.markets_by_token(&token_id).await?;
    if tm.condition_id != market.condition_id {
        return Err(PolymarketError::invalid(format!(
            "condition id mismatch: CLOB {} vs Gamma {} — refusing",
            tm.condition_id, market.condition_id
        )));
    }

    let tick_micro = clob.tick_size_micro(&token_id).await?;
    let book = clob.book(&token_id).await?;
    let min_size_micro = if book.min_order_size.is_empty() {
        0
    } else {
        order::parse_micro(&book.min_order_size)?
    };
    // Don't trust level ordering (the live CLOB returns asks descending and bids
    // ascending, i.e. best-last): compute the best price directly.
    let parse_levels = |levels: &[BookLevel]| -> Result<Vec<u64>> {
        levels
            .iter()
            .map(|l| order::parse_micro(&l.price))
            .collect()
    };
    let best_ask_micro = parse_levels(&book.asks)?.into_iter().min();
    let best_bid_micro = parse_levels(&book.bids)?.into_iter().max();
    let asks = book_levels(&book.asks, true)?;
    let bids = book_levels(&book.bids, false)?;

    Ok(Snapshot {
        market,
        token_id,
        neg_risk,
        tick_micro,
        min_size_micro,
        best_ask_micro,
        best_bid_micro,
        asks,
        bids,
    })
}

/// Resolve the limit price for this attempt. Marketable orders re-derive the
/// limit from the live book under the user's bound: the worst level needed to
/// fill `amount_micro`; explicit limit orders pin it.
pub fn choose_limit(
    side: Side,
    amount_micro: u64,
    marketable: bool,
    bound_micro: u64,
    pinned_limit_micro: u64,
    snap: &Snapshot,
) -> Result<u64> {
    if !marketable {
        return Ok(order::quantize_price(
            pinned_limit_micro,
            snap.tick_micro,
            side,
        ));
    }
    match side {
        Side::Buy => {
            let ask = snap
                .best_ask_micro
                .ok_or_else(|| PolymarketError::invalid("order book has no asks (no liquidity)"))?;
            if ask > bound_micro {
                return Err(PolymarketError::invalid(format!(
                    "best ask {} exceeds max price {} — refusing",
                    order::format_micro(ask),
                    order::format_micro(bound_micro)
                )));
            }
            let price = fill_price(Side::Buy, amount_micro, &snap.asks, bound_micro).unwrap_or(ask);
            Ok(order::quantize_marketable_limit(
                price,
                snap.tick_micro,
                Side::Buy,
            ))
        }
        Side::Sell => {
            let bid = snap
                .best_bid_micro
                .ok_or_else(|| PolymarketError::invalid("order book has no bids (no liquidity)"))?;
            if bid < bound_micro {
                return Err(PolymarketError::invalid(format!(
                    "best bid {} is below min price {} — refusing",
                    order::format_micro(bid),
                    order::format_micro(bound_micro)
                )));
            }
            let price =
                fill_price(Side::Sell, amount_micro, &snap.bids, bound_micro).unwrap_or(bid);
            let limit = order::quantize_marketable_limit(price, snap.tick_micro, Side::Sell);
            if limit < bound_micro {
                return Err(PolymarketError::invalid(format!(
                    "best bid {} is above min price {}, but tick quantization gives {} below \
                     your minimum — lower the min price or pin an explicit limit price",
                    order::format_micro(bid),
                    order::format_micro(bound_micro),
                    order::format_micro(limit)
                )));
            }
            Ok(limit)
        }
    }
}

/// Build the integer quote. `amount_micro` is the USD spend for buys and the
/// share size for sells. Market-class buys (FAK/FOK) use the CLOB's market-order
/// precision rules (maker ≤ 2 dp, taker ≤ 4 dp), which differ from limit orders.
pub fn build_quote(
    side: Side,
    amount_micro: u64,
    limit_micro: u64,
    snap: &Snapshot,
    order_type: OrderType,
) -> Result<LimitQuote> {
    let market_class = matches!(order_type, OrderType::FAK | OrderType::FOK);
    let quote = match (side, market_class) {
        (Side::Buy, true) => {
            LimitQuote::market_buy_from_spend(amount_micro, limit_micro, snap.tick_micro)
        }
        (Side::Buy, false) => {
            LimitQuote::buy_from_spend(amount_micro, limit_micro, snap.tick_micro)
        }
        (Side::Sell, _) => LimitQuote::sell_from_shares(amount_micro, limit_micro, snap.tick_micro),
    }?;
    if quote.size_micro < snap.min_size_micro {
        return Err(PolymarketError::invalid(format!(
            "order size {} shares is below the market minimum {} — increase the amount",
            order::format_micro(quote.size_micro),
            order::format_micro(snap.min_size_micro)
        )));
    }
    Ok(quote)
}

/// USD leg of a quote (what the per-order / daily policy caps look at).
pub fn usd_leg(q: &LimitQuote) -> u64 {
    match q.side {
        Side::Buy => q.maker_micro,
        Side::Sell => q.taker_micro,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draft_from_quote(
    wallet: &str,
    owner: String,
    funder: Option<String>,
    signature_type: u8,
    slug: &str,
    outcome: &str,
    side: Side,
    order_type: OrderType,
    price_bound_micro: u64,
    marketable: bool,
    book_snapshot_ms: u128,
    snap: &Snapshot,
    quote: &LimitQuote,
) -> OrderDraft {
    OrderDraft {
        id: String::new(),
        wallet: wallet.to_string(),
        owner,
        funder,
        signature_type,
        slug: slug.to_string(),
        question: snap.market.question.clone(),
        condition_id: snap.market.condition_id.clone(),
        outcome: outcome.to_ascii_uppercase(),
        token_id: snap.token_id.clone(),
        side,
        order_type,
        amount_microusd: usd_leg(quote),
        price_bound_micro,
        marketable,
        limit_price_micro: quote.price_micro,
        size_micro: quote.size_micro,
        tick_micro: snap.tick_micro,
        min_order_size_micro: snap.min_size_micro,
        neg_risk: snap.neg_risk,
        active: snap.market.active,
        closed: snap.market.closed,
        order_book_enabled: snap.market.enable_order_book,
        binary_outcomes: snap.market.is_binary(),
        best_ask_micro: snap.best_ask_micro,
        best_bid_micro: snap.best_bid_micro,
        book_snapshot_ms,
        policy_checks: serde_json::Value::Null,
        status: DraftStatus::Draft,
        salt: None,
        clob_order_id: None,
        clob_status: None,
        last_error: None,
        review_intent_hash: None,
        created_ms: 0,
        updated_ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(best_ask: Option<u64>, best_bid: Option<u64>) -> Snapshot {
        Snapshot {
            market: serde_json::from_value(serde_json::json!({})).unwrap(),
            token_id: "1".into(),
            neg_risk: false,
            tick_micro: 10_000,
            min_size_micro: 5_000_000,
            best_ask_micro: best_ask,
            best_bid_micro: best_bid,
            asks: vec![],
            bids: vec![],
        }
    }

    #[test]
    fn choose_limit_marketable_buy_respects_bound_and_quantizes() {
        let s = snap(Some(695_000), Some(690_000));
        assert!(choose_limit(Side::Buy, 0, true, 690_000, 0, &s).is_err());
        assert_eq!(
            choose_limit(Side::Buy, 0, true, 700_000, 0, &s).unwrap(),
            690_000
        );
    }

    #[test]
    fn choose_limit_marketable_sell_refuses_below_min() {
        let s = snap(Some(695_000), Some(690_000));
        assert!(choose_limit(Side::Sell, 0, true, 700_000, 0, &s).is_err());
        assert_eq!(
            choose_limit(Side::Sell, 0, true, 680_000, 0, &s).unwrap(),
            690_000
        );
    }

    /// The recorded ceasefire Yes book: 8.99 shares at 0.68 cover only $6.11
    /// of a $50 buy, so the order must reach 0.69, never past the bound.
    #[test]
    fn choose_limit_marketable_buy_reaches_the_level_that_fills_the_amount() {
        let mut s = snap(Some(680_000), Some(660_000));
        s.asks = vec![
            (680_000, 8_990_000),
            (690_000, 784_660_000),
            (700_000, 1_300_910_000),
        ];
        assert_eq!(
            choose_limit(Side::Buy, 50_000_000, true, 700_000, 0, &s).unwrap(),
            690_000
        );
        assert_eq!(
            choose_limit(Side::Buy, 5_000_000, true, 700_000, 0, &s).unwrap(),
            680_000
        );
        // A book too thin for the amount stops at the bound.
        assert_eq!(
            choose_limit(Side::Buy, 2_000_000_000, true, 690_000, 0, &s).unwrap(),
            690_000
        );
    }

    #[test]
    fn choose_limit_marketable_sell_reaches_the_level_that_fills_the_shares() {
        let mut s = snap(Some(700_000), Some(690_000));
        s.bids = vec![
            (690_000, 3_000_000),
            (680_000, 50_000_000),
            (600_000, 50_000_000),
        ];
        assert_eq!(
            choose_limit(Side::Sell, 20_000_000, true, 650_000, 0, &s).unwrap(),
            680_000
        );
        assert_eq!(
            choose_limit(Side::Sell, 200_000_000, true, 650_000, 0, &s).unwrap(),
            680_000
        );
    }

    #[test]
    fn book_levels_sort_best_first() {
        let level = |price: &str| crate::polymarket::types::BookLevel {
            price: price.into(),
            size: "1".into(),
        };
        let book = [level("0.85"), level("0.68"), level("0.7")];
        let prices = |ask| {
            book_levels(&book, ask)
                .unwrap()
                .into_iter()
                .map(|(p, _)| p)
                .collect::<Vec<_>>()
        };
        assert_eq!(prices(true), [680_000, 700_000, 850_000]);
        assert_eq!(prices(false), [850_000, 700_000, 680_000]);
    }

    #[test]
    fn choose_limit_pinned_quantizes_without_book() {
        let s = snap(None, None);
        assert_eq!(
            choose_limit(Side::Buy, 0, false, 0, 695_500, &s).unwrap(),
            690_000
        );
    }

    #[test]
    fn build_quote_enforces_min_size() {
        let s = snap(Some(500_000), Some(490_000));
        assert!(build_quote(Side::Buy, 2_000_000, 500_000, &s, OrderType::FAK).is_err());
        let q = build_quote(Side::Buy, 5_000_000, 500_000, &s, OrderType::FAK).unwrap();
        assert!(q.size_micro >= s.min_size_micro);
        assert_eq!(usd_leg(&q), q.maker_micro);
    }
}
