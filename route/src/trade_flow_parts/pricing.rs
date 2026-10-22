use crate::prelude::*;

use crate::polymarket::order::{LimitQuote, OrderType, parse_micro};
use crate::polymarket::trade as shared_trade;
use crate::polymarket::types::BookLevel;
use crate::polymarket::{Market, OrderBook, Result, Side};
use petal::sdk::DispatchResponse;
pub fn trade_snapshot(slug: &str, outcome: &str) -> Result<TradeSnapshot, DispatchResponse> {
    let market: Market = get_json(&format!(
        "{}/markets/slug/{slug}",
        crate::runtime_config::gamma_url()
    ))?;
    if !market.is_binary() {
        return Err(error(
            -3,
            format!("market '{slug}' is not a binary YES/NO market"),
        ));
    }
    if !market.active {
        return Err(error(-3, format!("market '{slug}' is not active")));
    }
    if market.closed {
        return Err(error(-3, format!("market '{slug}' is closed")));
    }
    if !market.enable_order_book {
        return Err(error(
            -3,
            format!("market '{slug}' does not have the order book enabled"),
        ));
    }
    let outcome = match outcome.to_ascii_uppercase().as_str() {
        "YES" => "YES",
        "NO" => "NO",
        other => return Err(error(-3, format!("outcome must be YES or NO, got {other}"))),
    };
    let token_id = match outcome {
        "YES" => market.yes_token_id(),
        "NO" => market.no_token_id(),
        _ => None,
    }
    .ok_or_else(|| error(-3, format!("market '{slug}' has no {outcome} token id")))?
    .to_string();
    let book: OrderBook = get_json(&url_with_query(
        &format!("{}/book", crate::runtime_config::clob_url()),
        &[("token_id", &token_id)],
    ))?;
    if !book.asset_id.is_empty() && book.asset_id != token_id {
        return Err(error(
            -4,
            format!(
                "CLOB book token mismatch: requested {token_id}, received {}",
                book.asset_id
            ),
        ));
    }
    if !book.market.is_empty()
        && !market.condition_id.is_empty()
        && book.market != market.condition_id
    {
        return Err(error(
            -4,
            format!(
                "CLOB book condition mismatch: Gamma {} vs CLOB {}",
                market.condition_id, book.market
            ),
        ));
    }
    if book.neg_risk != market.neg_risk {
        return Err(error(
            -4,
            format!(
                "neg_risk mismatch for '{slug}': Gamma={} CLOB={}",
                market.neg_risk, book.neg_risk
            ),
        ));
    }
    let tick_micro = if book.tick_size.trim().is_empty() {
        match market.order_price_min_tick_size {
            Some(tick) => parse_api_float_micro(tick, "orderPriceMinTickSize")?,
            None => return Err(error(-4, "CLOB book omitted tick_size")),
        }
    } else {
        parse_micro(&book.tick_size).map_err(|e| error(-4, e.to_string()))?
    };
    let min_size_micro = if book.min_order_size.trim().is_empty() {
        match market.order_min_size {
            Some(size) => parse_api_float_micro(size, "orderMinSize")?,
            None => 0,
        }
    } else {
        parse_micro(&book.min_order_size).map_err(|e| error(-4, e.to_string()))?
    };
    let best_ask_micro = best_price(&book.asks, true)?;
    let best_bid_micro = best_price(&book.bids, false)?;
    Ok(TradeSnapshot {
        market,
        outcome: outcome.into(),
        token_id,
        neg_risk: book.neg_risk,
        tick_micro,
        min_size_micro,
        best_ask_micro,
        best_bid_micro,
        active: true,
        closed: false,
        order_book_enabled: true,
    })
}

pub fn best_price(levels: &[BookLevel], ask: bool) -> Result<Option<u64>, DispatchResponse> {
    let mut best: Option<u64> = None;
    for level in levels {
        let price = parse_micro(&level.price).map_err(|e| error(-4, e.to_string()))?;
        best = Some(match best {
            None => price,
            Some(existing) if ask => existing.min(price),
            Some(existing) => existing.max(price),
        });
    }
    Ok(best)
}

pub fn choose_trade_limit(
    side: Side,
    marketable: bool,
    bound_micro: u64,
    pinned_limit_micro: u64,
    snapshot: &TradeSnapshot,
) -> Result<u64, DispatchResponse> {
    shared_trade::choose_limit(
        side,
        marketable,
        bound_micro,
        pinned_limit_micro,
        &snapshot.as_shared(),
    )
    .map_err(polymarket_error)
}

pub fn build_trade_quote(
    side: Side,
    amount_micro: u64,
    limit_micro: u64,
    snapshot: &TradeSnapshot,
    order_type: OrderType,
) -> Result<LimitQuote, DispatchResponse> {
    build_trade_quote_with_fees(side, amount_micro, limit_micro, snapshot, order_type, None)
}

/// As [`build_trade_quote`], sizing a market buy so its fees fit the spend.
///
/// Every stage that quotes a draft must pass the same fee inputs. Draft,
/// revalidate and post all resolve them through
/// [`resolve_spend_fee_inputs`], so their numbers agree and
/// `review_intent_matches_draft` does not fail on a quote that moved.
pub fn build_trade_quote_with_fees(
    side: Side,
    amount_micro: u64,
    limit_micro: u64,
    snapshot: &TradeSnapshot,
    order_type: OrderType,
    fees: Option<SpendFeeInputs>,
) -> Result<LimitQuote, DispatchResponse> {
    let pricer = fees.map(|fees| {
        move |notional: u64| -> crate::polymarket::Result<u64> {
            fees.total_micro(notional, limit_micro)
                .map_err(crate::polymarket::PolymarketError::invalid)
        }
    });
    shared_trade::build_quote(
        side,
        amount_micro,
        limit_micro,
        &snapshot.as_shared(),
        order_type,
        pricer
            .as_ref()
            .map(|pricer| pricer as &dyn Fn(u64) -> crate::polymarket::Result<u64>),
    )
    .map_err(polymarket_error)
}

/// The fee inputs a market buy is sized against.
#[derive(Clone, Copy, Debug)]
pub struct SpendFeeInputs {
    pub builder_bps: u16,
    pub platform: Option<crate::builder_fee::PlatformFeeParams>,
}

impl SpendFeeInputs {
    fn total_micro(self, notional_micro: u64, price_micro: u64) -> Result<u64, String> {
        let builder =
            crate::builder_code::builder_fee_ceiling_micro(notional_micro, self.builder_bps)?;
        let platform = match self.platform {
            Some(params) => params.ceiling_micro(notional_micro, price_micro)?,
            None => 0,
        };
        Ok(builder.saturating_add(platform))
    }
}

/// Resolves the fee inputs for a draft, or `None` when no builder code is
/// configured. One place, so every stage that quotes agrees.
pub fn resolve_spend_fee_inputs(
    condition_id: &str,
    order_type: OrderType,
) -> Result<Option<SpendFeeInputs>, DispatchResponse> {
    let Some(attribution) = crate::account_views::resolve_builder_attribution()? else {
        return Ok(None);
    };
    let builder_bps =
        crate::builder_fee::FeeRateBasis::for_order_type(order_type).bps(attribution.ceiling);
    Ok(Some(SpendFeeInputs {
        builder_bps,
        platform: crate::builder_fee::read_platform_fee_params(condition_id),
    }))
}

pub fn trade_policy_check(
    wallet: &str,
    draft: &StoreTradeDraft,
) -> Result<serde_json::Value, DispatchResponse> {
    let policy = venue_config(wallet)?;
    let (receipt_store_readable, daily_posted_microusd) = daily_posted_microusd(wallet);
    let ctx = LocalPolymarketOrderCtx {
        slug: draft.slug.clone(),
        condition_id: draft.condition_id.clone(),
        side: match draft.side {
            Side::Buy => LocalPolicySide::Buy,
            Side::Sell => LocalPolicySide::Sell,
        },
        amount_microusd: draft.amount_micro,
        limit_price_micro: draft.limit_price_micro,
        active: draft.active,
        closed: draft.closed,
        order_book_enabled: draft.order_book_enabled,
        binary_outcomes: draft.binary_outcomes,
        neg_risk: draft.neg_risk,
        receipt_store_readable,
        daily_posted_microusd,
    };
    let checks = evaluate_local_polymarket_order(&policy, &ctx);
    let deny = local_policy_has_deny(&checks);
    let warn = local_policy_has_warn(&checks);
    let policy_status = if deny {
        "denied"
    } else if warn {
        "warn"
    } else {
        "passed"
    };
    let posting_enabled = !deny && draft.side == Side::Buy && draft.order_type != OrderType::GTD;
    let reason = if draft.side == Side::Sell {
        "sell posting requires passing authoritative chain CTF balance and approval checks"
    } else if draft.order_type == OrderType::GTD {
        "GTD posting is disabled until expiry parity is ported"
    } else {
        "buy can be posted after final review by writing to the post endpoint; resting GTC orders can be cancelled from their receipt"
    };
    Ok(serde_json::json!({
        "status": "blocked",
        "reason": reason,
        "policy_status": policy_status,
        "policy_deny": deny,
        "policy_warn": warn,
        "policy_checks": checks,
        "receipt_store_readable": receipt_store_readable,
        "daily_posted_microusd": daily_posted_microusd,
        "receipt_audit_parity": true,
        "active": draft.active,
        "closed": draft.closed,
        "binary_outcomes": draft.binary_outcomes,
        "order_book_enabled": draft.order_book_enabled,
        "size_at_or_above_min": draft.size_micro >= draft.min_order_size_micro,
        "signing_enabled": posting_enabled,
        "posting_enabled": posting_enabled
    }))
}
