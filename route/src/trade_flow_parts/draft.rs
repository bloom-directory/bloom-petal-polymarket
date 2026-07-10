use crate::prelude::*;

use crate::polymarket::order::{OrderType, format_micro, parse_micro};
use crate::polymarket::{Side, validate_wallet_name};
use petal::sdk::DispatchResponse;
pub(crate) fn create_trade_draft(wallet: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let req: TradeNewRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("trade new JSON: {e}")),
    };
    let side = match req
        .side
        .as_deref()
        .unwrap_or("buy")
        .to_ascii_lowercase()
        .as_str()
    {
        "buy" => Side::Buy,
        "sell" => Side::Sell,
        other => return error(-3, format!("side must be buy or sell, got {other}")),
    };
    let amount_micro = match parse_micro(req.amount.trim()) {
        Ok(value) if value > 0 => value,
        Ok(_) => return error(-3, "amount must be > 0"),
        Err(e) => return error(-3, e.to_string()),
    };
    let bound = match side {
        Side::Buy => req.max_price.as_ref().or(req.limit_price.as_ref()),
        Side::Sell => req.min_price.as_ref().or(req.limit_price.as_ref()),
    };
    let Some(bound) = bound else {
        return error(
            -3,
            match side {
                Side::Buy => "buy requires max_price or limit_price",
                Side::Sell => "sell requires min_price or limit_price",
            },
        );
    };
    let bound_micro = match parse_micro(bound.trim()) {
        Ok(value) if value > 0 => value,
        Ok(_) => return error(-3, "price bound must be > 0"),
        Err(e) => return error(-3, e.to_string()),
    };
    let order_type = match req.order_type.as_deref() {
        Some(raw) => match raw.parse::<OrderType>() {
            Ok(OrderType::GTD) => return error(-3, "GTD orders are not supported"),
            Ok(value) => value,
            Err(e) => return error(-3, e.to_string()),
        },
        None if req.limit_price.is_some() => OrderType::GTC,
        None => OrderType::FAK,
    };
    let snapshot = match trade_snapshot(&req.slug, &req.outcome) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    let marketable = req.limit_price.is_none();
    let pinned_limit_micro = match req.limit_price.as_deref() {
        Some(limit) => match parse_micro(limit.trim()) {
            Ok(value) if value > 0 => value,
            Ok(_) => return error(-3, "limit_price must be > 0"),
            Err(e) => return error(-3, e.to_string()),
        },
        None => bound_micro,
    };
    if !marketable {
        match side {
            Side::Buy if pinned_limit_micro > bound_micro => {
                return error(-3, "limit_price exceeds max_price");
            }
            Side::Sell if pinned_limit_micro < bound_micro => {
                return error(-3, "limit_price is below min_price");
            }
            _ => {}
        }
    }
    let limit_micro =
        match choose_trade_limit(side, marketable, bound_micro, pinned_limit_micro, &snapshot) {
            Ok(limit) => limit,
            Err(resp) => return resp,
        };
    let quote = match build_trade_quote(side, amount_micro, limit_micro, &snapshot, order_type) {
        Ok(quote) => quote,
        Err(resp) => return resp,
    };
    let id = next_id(&format!("trade/{wallet}/drafts/"), "/order.json");
    let draft = StoreTradeDraft {
        id: id.clone(),
        wallet: wallet.into(),
        slug: req.slug,
        question: snapshot.market.question,
        condition_id: snapshot.market.condition_id,
        outcome: snapshot.outcome,
        token_id: snapshot.token_id,
        side,
        order_type,
        amount_micro,
        price_bound_micro: bound_micro,
        limit_price: req.limit_price,
        marketable,
        limit_price_micro: quote.price_micro,
        size_micro: quote.size_micro,
        maker_micro: quote.maker_micro,
        taker_micro: quote.taker_micro,
        tick_micro: snapshot.tick_micro,
        min_order_size_micro: snapshot.min_size_micro,
        neg_risk: snapshot.neg_risk,
        active: snapshot.active,
        closed: snapshot.closed,
        order_book_enabled: snapshot.order_book_enabled,
        binary_outcomes: true,
        best_ask_micro: snapshot.best_ask_micro,
        best_bid_micro: snapshot.best_bid_micro,
        book_snapshot_secs: now_secs(),
        status: "review".into(),
        salt: None,
        clob_order_id: None,
        clob_status: None,
        last_error: None,
    };
    let policy_check = match trade_policy_check(wallet, &draft) {
        Ok(check) => check,
        Err(resp) => return resp,
    };
    let base = format!("trade/{wallet}/drafts/{id}");
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/order.json"), &draft, false)
    {
        return error(-4, "failed to store draft");
    }
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
    {
        return error(-4, "failed to store policy check");
    }
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/quote.json"),
        &serde_json::json!({
            "side": draft.side,
            "order_type": draft.order_type.as_str(),
            "marketable": draft.marketable,
            "amount_micro": draft.amount_micro,
            "amount": format_micro(draft.amount_micro),
            "price_bound_micro": draft.price_bound_micro,
            "price_bound": format_micro(draft.price_bound_micro),
            "limit_price_micro": draft.limit_price_micro,
            "limit_price": format_micro(draft.limit_price_micro),
            "size_micro": draft.size_micro,
            "size": format_micro(draft.size_micro),
            "maker_micro": draft.maker_micro,
            "maker": format_micro(draft.maker_micro),
            "taker_micro": draft.taker_micro,
            "taker": format_micro(draft.taker_micro),
            "tick_micro": draft.tick_micro,
            "tick": format_micro(draft.tick_micro),
            "min_order_size_micro": draft.min_order_size_micro,
            "min_order_size": format_micro(draft.min_order_size_micro),
            "best_ask_micro": draft.best_ask_micro,
            "best_bid_micro": draft.best_bid_micro,
            "status": "quoted"
        }),
        false,
    ) {
        return error(-4, "failed to store quote");
    }
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/review_intent.json"),
        &serde_json::json!({
            "wallet": wallet,
            "draft_id": id,
            "slug": draft.slug,
            "outcome": draft.outcome,
            "token_id": draft.token_id,
            "limit_price": format_micro(draft.limit_price_micro),
            "size": format_micro(draft.size_micro),
            "status": "created"
        }),
        false,
    ) {
        return error(-4, "failed to store review intent");
    }
    DispatchResponse::Write
}
