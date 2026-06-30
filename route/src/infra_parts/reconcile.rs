use crate::prelude::*;

use crate::polymarket::order::{OrderType, parse_micro};
use crate::polymarket::{Credentials, Result, Side};
use alloy::primitives::Address;
pub(crate) fn clob_response_status(raw: &serde_json::Value) -> String {
    raw.get("status")
        .or_else(|| raw.get("orderStatus"))
        .or_else(|| raw.get("order_status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("posted")
        .to_ascii_lowercase()
}

pub(crate) fn clob_status_excluded_from_daily_cap(
    status: &str,
    order_type: Option<OrderType>,
) -> bool {
    status == "rejected"
        || (status == "unmatched" && order_type.is_some_and(|order_type| !order_type.can_rest()))
}

pub(crate) fn reconcile_ambiguous_post(
    owner: Address,
    creds: &Credentials,
    draft: &StoreTradeDraft,
    funder: Address,
    salt: u64,
) -> Option<serde_json::Value> {
    let open_orders = clob_l2_get_json(owner, creds, "/data/orders", &[]).ok()?;
    find_matching_open_order(&open_orders, draft, funder, salt)
}

pub(crate) fn find_matching_open_order(
    raw: &serde_json::Value,
    draft: &StoreTradeDraft,
    funder: Address,
    salt: u64,
) -> Option<serde_json::Value> {
    match raw {
        serde_json::Value::Array(items) => items
            .iter()
            .find(|item| open_order_matches_draft(item, draft, funder, salt))
            .cloned(),
        serde_json::Value::Object(map) => ["orders", "data", "results"]
            .iter()
            .filter_map(|key| map.get(*key))
            .find_map(|value| find_matching_open_order(value, draft, funder, salt)),
        _ => None,
    }
}

pub(crate) fn open_order_matches_draft(
    item: &serde_json::Value,
    draft: &StoreTradeDraft,
    funder: Address,
    salt: u64,
) -> bool {
    if clob_response_order_id(item).is_none() {
        return false;
    }
    if matches!(
        clob_response_status(item).as_str(),
        "rejected" | "cancelled" | "canceled"
    ) {
        return false;
    }
    let Some(salts) = (match clob_order_field_u64s(item, &["salt"]) {
        Ok(values) => values,
        Err(()) => return false,
    }) else {
        return false;
    };
    if salts.iter().any(|value| *value != salt) {
        return false;
    }

    let mut matched_fields = 0usize;
    if let Some(values) = match clob_order_field_strings(
        item,
        &["asset_id", "assetId", "token_id", "tokenId", "tokenID"],
    ) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        if values.iter().any(|value| value != &draft.token_id) {
            return false;
        }
        matched_fields += 1;
    }
    if let Some(values) = match clob_order_field_strings(item, &["maker", "signer", "funder"]) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        let expected = funder.to_checksum(None);
        if values
            .iter()
            .any(|value| !address_strings_equal(value, &expected))
        {
            return false;
        }
        matched_fields += 1;
    }
    if clob_order_fields(item, &["side"])
        .into_iter()
        .try_fold(false, |_, value| clob_side_value_matches(value, draft.side))
        .unwrap_or(false)
    {
        matched_fields += 1;
    } else if !clob_order_fields(item, &["side"]).is_empty() {
        return false;
    }
    if let Some(values) = match clob_order_field_strings(item, &["orderType", "order_type"]) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        if values
            .iter()
            .any(|value| !value.eq_ignore_ascii_case(draft.order_type.as_str()))
        {
            return false;
        }
        matched_fields += 1;
    }
    if let Some(values) = match clob_order_field_u64s(item, &["makerAmount", "maker_amount"]) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        if values.iter().any(|value| *value != draft.maker_micro) {
            return false;
        }
        matched_fields += 1;
    }
    if let Some(values) = match clob_order_field_u64s(item, &["takerAmount", "taker_amount"]) {
        Ok(values) => values,
        Err(()) => return false,
    } {
        if values.iter().any(|value| *value != draft.taker_micro) {
            return false;
        }
        matched_fields += 1;
    }

    matched_fields >= 2
}

pub(crate) fn clob_order_fields<'a>(
    item: &'a serde_json::Value,
    names: &[&str],
) -> Vec<&'a serde_json::Value> {
    let mut values = Vec::new();
    for name in names {
        if let Some(value) = item.get(*name) {
            values.push(value);
        }
    }
    if let Some(order) = item.get("order") {
        for name in names {
            if let Some(value) = order.get(*name) {
                values.push(value);
            }
        }
    }
    values
}

pub(crate) fn clob_order_field_strings(
    item: &serde_json::Value,
    names: &[&str],
) -> Result<Option<Vec<String>>, ()> {
    let mut values = Vec::new();
    for value in clob_order_fields(item, names) {
        match value {
            serde_json::Value::String(s) => values.push(s.clone()),
            serde_json::Value::Number(n) => values.push(n.to_string()),
            _ => return Err(()),
        }
    }
    Ok((!values.is_empty()).then_some(values))
}

pub(crate) fn clob_order_field_u64s(
    item: &serde_json::Value,
    names: &[&str],
) -> Result<Option<Vec<u64>>, ()> {
    let mut values = Vec::new();
    for value in clob_order_fields(item, names) {
        let Some(parsed) = (match value {
            serde_json::Value::Number(n) => n.as_u64(),
            serde_json::Value::String(s) => s.parse::<u64>().ok(),
            _ => None,
        }) else {
            return Err(());
        };
        values.push(parsed);
    }
    Ok((!values.is_empty()).then_some(values))
}

pub(crate) fn address_strings_equal(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

pub(crate) fn clob_side_value_matches(value: &serde_json::Value, side: Side) -> Result<bool, bool> {
    let matches = match value {
        serde_json::Value::String(s) => {
            let normalized = s.trim().to_ascii_uppercase();
            match side {
                Side::Buy => normalized == "BUY" || normalized == "0",
                Side::Sell => normalized == "SELL" || normalized == "1",
            }
        }
        serde_json::Value::Number(n) => n
            .as_u64()
            .is_some_and(|value| matches!((value, side), (0, Side::Buy) | (1, Side::Sell))),
        _ => return Err(false),
    };
    matches.then_some(true).ok_or(false)
}

pub(crate) fn clob_response_order_id(raw: &serde_json::Value) -> Option<String> {
    raw.get("orderID")
        .or_else(|| raw.get("orderId"))
        .or_else(|| raw.get("order_id"))
        .or_else(|| raw.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

pub(crate) fn clob_response_filled_size_micro(raw: &serde_json::Value) -> Option<u64> {
    raw.get("size_matched")
        .or_else(|| raw.get("matched_size"))
        .or_else(|| raw.get("filled_size"))
        .and_then(|value| match value {
            serde_json::Value::String(s) => parse_micro(s).ok(),
            serde_json::Value::Number(n) => n
                .as_f64()
                .and_then(|f| parse_api_float_micro(f, "filled_size").ok()),
            _ => None,
        })
}

pub(crate) fn clob_cancel_confirmed(raw: &serde_json::Value, order_id: &str) -> bool {
    let status_cancelled = raw
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(|status| {
            let status = status.to_ascii_lowercase();
            status == "cancelled" || status == "canceled"
        })
        .unwrap_or(false);
    let status_order_matches = raw
        .get("orderID")
        .or_else(|| raw.get("orderId"))
        .or_else(|| raw.get("order_id"))
        .or_else(|| raw.get("id"))
        .and_then(serde_json::Value::as_str)
        == Some(order_id);
    let listed_cancelled = raw
        .get("canceled")
        .or_else(|| raw.get("cancelled"))
        .and_then(serde_json::Value::as_array)
        .map(|items| items.iter().any(|item| item.as_str() == Some(order_id)))
        .unwrap_or(false);
    listed_cancelled || (status_cancelled && status_order_matches)
}

pub(crate) fn clob_response_public_summary(
    status: &str,
    order_id: &Option<String>,
    filled_size_micro: Option<u64>,
) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "order_id": order_id,
        "filled_size_micro": filled_size_micro,
        "response_redacted": true
    })
}

pub(crate) fn clob_reconciled_public_summary(
    status: &str,
    order_id: &Option<String>,
    filled_size_micro: Option<u64>,
) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "order_id": order_id,
        "filled_size_micro": filled_size_micro,
        "reconciled_from": "open_orders",
        "response_redacted": true
    })
}

pub(crate) fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
