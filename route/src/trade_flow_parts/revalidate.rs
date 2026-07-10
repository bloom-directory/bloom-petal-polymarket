use crate::prelude::*;

use crate::polymarket::order::{OrderType, format_micro};
use crate::polymarket::{Result, Side, validate_wallet_name};
use crate::trade_flow_parts::policy::enable_trade_posting;
use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HostStatus, SdkError};
pub(crate) fn revalidate_trade_draft(wallet: &str, id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_segment(id) {
        return error(-3, "invalid draft id");
    }
    let req: TradeRevalidateRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("revalidate JSON: {e}")),
    };
    if !req.revalidate {
        return error(-3, "revalidate must be true");
    }
    let base = format!("trade/{wallet}/drafts/{id}");
    let _lock = match acquire_trade_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let mut draft = match load_trade_draft(wallet, id) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    if draft.status != "review" && draft.status != "revalidated" {
        return error(
            -3,
            format!("draft {id} is '{}' and cannot be revalidated", draft.status),
        );
    }
    if draft.order_type == OrderType::GTD {
        return error(-3, "posting GTD orders is pending expiry parity");
    }

    let snapshot = match trade_snapshot(&draft.slug, &draft.outcome) {
        Ok(snapshot) => snapshot,
        Err(resp) => return resp,
    };
    if snapshot.token_id != draft.token_id {
        return error(
            -3,
            "token id changed between draft and revalidate; refusing",
        );
    }
    if snapshot.market.condition_id != draft.condition_id {
        return error(
            -3,
            "condition id changed between draft and revalidate; refusing",
        );
    }
    if snapshot.neg_risk != draft.neg_risk {
        return error(
            -3,
            "neg-risk changed between draft and revalidate; refusing",
        );
    }
    let amount_input = match draft.side {
        Side::Buy => draft.amount_micro.max(1),
        Side::Sell => draft.size_micro,
    };
    let limit_micro = match choose_trade_limit(
        draft.side,
        draft.marketable,
        draft.price_bound_micro,
        draft.limit_price_micro,
        &snapshot,
    ) {
        Ok(limit) => limit,
        Err(resp) => return resp,
    };
    let quote = match build_trade_quote(
        draft.side,
        amount_input,
        limit_micro,
        &snapshot,
        draft.order_type,
    ) {
        Ok(quote) => quote,
        Err(resp) => return resp,
    };

    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let funder = match tradeable_deposit_wallet(wallet, owner) {
        Ok(funder) => funder,
        Err(resp) => return resp,
    };

    draft.limit_price_micro = quote.price_micro;
    draft.size_micro = quote.size_micro;
    draft.maker_micro = quote.maker_micro;
    draft.taker_micro = quote.taker_micro;
    if draft.side == Side::Sell {
        draft.amount_micro = draft.taker_micro;
    }
    draft.tick_micro = snapshot.tick_micro;
    draft.min_order_size_micro = snapshot.min_size_micro;
    draft.neg_risk = snapshot.neg_risk;
    draft.active = snapshot.active;
    draft.closed = snapshot.closed;
    draft.order_book_enabled = snapshot.order_book_enabled;
    draft.binary_outcomes = snapshot.market.is_binary();
    draft.best_ask_micro = snapshot.best_ask_micro;
    draft.best_bid_micro = snapshot.best_bid_micro;
    draft.book_snapshot_secs = now_secs();
    draft.status = "revalidated".into();
    let mut policy_check = match trade_policy_check(wallet, &draft) {
        Ok(check) => check,
        Err(resp) => return resp,
    };
    let policy_deny = policy_check
        .get("policy_deny")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let policy_status = policy_check
        .get("policy_status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
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
            "status": "revalidated"
        }),
        false,
    ) {
        return error(-4, "failed to store quote");
    }
    if policy_deny {
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
        {
            return error(-4, "failed to store policy check");
        }
        match petal::sdk::store_del(&format!("{base}/review_intent.json")) {
            Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => {}
            Err(_) => return error(-4, "failed to clear stale review intent"),
        }
        draft.status = "policy_denied".into();
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/order.json"), &draft, false)
        {
            return error(-4, "failed to store denied draft");
        }
        return error(-3, "Polymarket policy denied; see policy_check.json");
    }
    let sell_preflight = if draft.side == Side::Sell {
        match verify_sell_preflight(
            wallet,
            owner,
            funder,
            &draft.token_id,
            draft.size_micro,
            draft.neg_risk,
        ) {
            Ok(preflight) => {
                enable_trade_posting(
                    &mut policy_check,
                    "sell can be posted after final review because chain CTF balance and approval checks passed",
                );
                Some(preflight)
            }
            Err(resp) => {
                match petal::sdk::store_del(&format!("{base}/review_intent.json")) {
                    Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => {}
                    Err(_) => return error(-4, "failed to clear stale review intent"),
                }
                draft.status = "preflight_denied".into();
                if let DispatchResponse::Error { .. } =
                    store_put_json(&format!("{base}/order.json"), &draft, false)
                {
                    return error(-4, "failed to store denied draft");
                }
                return resp;
            }
        }
    } else {
        None
    };
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
    {
        return error(-4, "failed to store policy check");
    }
    let posting_enabled = policy_check
        .get("posting_enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/review_intent.json"),
        &serde_json::json!({
            "wallet": wallet,
            "draft_id": id,
            "owner": owner.to_checksum(None),
            "funder": funder.to_checksum(None),
            "slug": draft.slug,
            "condition_id": draft.condition_id,
            "outcome": draft.outcome,
            "token_id": draft.token_id,
            "side": draft.side,
            "order_type": draft.order_type.as_str(),
            "limit_price": format_micro(draft.limit_price_micro),
            "size": format_micro(draft.size_micro),
            "maker": format_micro(draft.maker_micro),
            "taker": format_micro(draft.taker_micro),
            "neg_risk": draft.neg_risk,
            "policy_status": policy_status,
            "sell_preflight": sell_preflight,
            "status": "final_review_staged",
            "signing_enabled": posting_enabled,
            "posting_enabled": posting_enabled
        }),
        false,
    ) {
        return error(-4, "failed to store review intent");
    }
    store_put_json(&format!("{base}/order.json"), &draft, false)
}

pub(crate) fn refresh_trade_post_inputs(
    wallet: &str,
    base: &str,
    draft: &mut StoreTradeDraft,
    owner: Address,
) -> Result<(serde_json::Value, Option<serde_json::Value>), DispatchResponse> {
    let snapshot = trade_snapshot(&draft.slug, &draft.outcome)?;
    if snapshot.token_id != draft.token_id {
        return Err(error(
            -3,
            "token id changed between draft and post; refusing",
        ));
    }
    if snapshot.market.condition_id != draft.condition_id {
        return Err(error(
            -3,
            "condition id changed between draft and post; refusing",
        ));
    }
    if snapshot.neg_risk != draft.neg_risk {
        return Err(error(
            -3,
            "neg-risk changed between draft and post; refusing",
        ));
    }
    let amount_input = match draft.side {
        Side::Buy => draft.amount_micro.max(1),
        Side::Sell => draft.size_micro,
    };
    let limit_micro = choose_trade_limit(
        draft.side,
        draft.marketable,
        draft.price_bound_micro,
        draft.limit_price_micro,
        &snapshot,
    )?;
    let quote = build_trade_quote(
        draft.side,
        amount_input,
        limit_micro,
        &snapshot,
        draft.order_type,
    )?;
    draft.limit_price_micro = quote.price_micro;
    draft.size_micro = quote.size_micro;
    draft.maker_micro = quote.maker_micro;
    draft.taker_micro = quote.taker_micro;
    if draft.side == Side::Sell {
        draft.amount_micro = draft.taker_micro;
    }
    draft.tick_micro = snapshot.tick_micro;
    draft.min_order_size_micro = snapshot.min_size_micro;
    draft.neg_risk = snapshot.neg_risk;
    draft.active = snapshot.active;
    draft.closed = snapshot.closed;
    draft.order_book_enabled = snapshot.order_book_enabled;
    draft.binary_outcomes = snapshot.market.is_binary();
    draft.best_ask_micro = snapshot.best_ask_micro;
    draft.best_bid_micro = snapshot.best_bid_micro;
    draft.book_snapshot_secs = now_secs();
    let mut policy_check = trade_policy_check(wallet, draft)?;
    let policy_deny = policy_check
        .get("policy_deny")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
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
            "status": "post_revalidated"
        }),
        false,
    ) {
        return Err(error(-4, "failed to store quote"));
    }
    if policy_deny {
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
        {
            return Err(error(-4, "failed to store policy check"));
        }
        match petal::sdk::store_del(&format!("{base}/review_intent.json")) {
            Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => {}
            Err(_) => return Err(error(-4, "failed to clear stale review intent")),
        }
        draft.status = "policy_denied".into();
        if let DispatchResponse::Error { .. } =
            store_put_json(&format!("{base}/order.json"), draft, false)
        {
            return Err(error(-4, "failed to store denied draft"));
        }
        return Err(error(-3, "Polymarket policy denied; see policy_check.json"));
    }
    let sell_preflight = if draft.side == Side::Sell {
        let funder = tradeable_deposit_wallet(wallet, owner)?;
        let preflight = verify_sell_preflight(
            wallet,
            owner,
            funder,
            &draft.token_id,
            draft.size_micro,
            draft.neg_risk,
        )?;
        enable_trade_posting(
            &mut policy_check,
            "sell can be posted after final review because chain CTF balance and approval checks passed",
        );
        Some(preflight)
    } else {
        None
    };
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/policy_check.json"), &policy_check, false)
    {
        return Err(error(-4, "failed to store policy check"));
    }
    Ok((policy_check, sell_preflight))
}

pub(crate) fn review_intent_matches_draft(
    review: &serde_json::Value,
    draft: &StoreTradeDraft,
    owner: Address,
    funder: Address,
    policy_check: &serde_json::Value,
    sell_preflight: Option<&serde_json::Value>,
) -> Result<(), String> {
    let side = match draft.side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    };
    let policy_status = policy_check
        .get("policy_status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    for (field, expected) in [
        ("wallet", draft.wallet.clone()),
        ("draft_id", draft.id.clone()),
        ("owner", owner.to_checksum(None)),
        ("funder", funder.to_checksum(None)),
        ("slug", draft.slug.clone()),
        ("condition_id", draft.condition_id.clone()),
        ("outcome", draft.outcome.clone()),
        ("token_id", draft.token_id.clone()),
        ("side", side.to_string()),
        ("order_type", draft.order_type.as_str().to_string()),
        ("limit_price", format_micro(draft.limit_price_micro)),
        ("size", format_micro(draft.size_micro)),
        ("maker", format_micro(draft.maker_micro)),
        ("taker", format_micro(draft.taker_micro)),
        ("policy_status", policy_status.to_string()),
    ] {
        if review.get(field).and_then(serde_json::Value::as_str) != Some(expected.as_str()) {
            return Err(format!(
                "final review field '{field}' no longer matches live post inputs"
            ));
        }
    }
    if review.get("neg_risk").and_then(serde_json::Value::as_bool) != Some(draft.neg_risk) {
        return Err("final review field 'neg_risk' no longer matches live post inputs".into());
    }
    if draft.side == Side::Sell {
        let Some(fresh) = sell_preflight else {
            return Err("final review field 'sell_preflight' is missing live post evidence".into());
        };
        if review.get("sell_preflight") != Some(fresh) {
            return Err(
                "final review field 'sell_preflight' no longer matches live post inputs".into(),
            );
        }
    }
    Ok(())
}
