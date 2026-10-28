use crate::prelude::*;
use std::collections::BTreeSet;

use crate::polymarket::eip712::{CTF, CTF_EXCHANGE, NEG_RISK_EXCHANGE};
use crate::polymarket::fees::max_buy_fee_cap;
use crate::polymarket::order::{format_micro, parse_micro};
use crate::polymarket::{Position, Result, Side};
use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HostStatus, SdkError};

/// Enable posting for a sell only from a passing preflight taken for this
/// draft's deposit wallet, token and size. A quote alone never enables a sell.
pub fn enable_sell_posting(
    policy_check: &mut serde_json::Value,
    draft: &StoreTradeDraft,
    deposit: Address,
    preflight: &serde_json::Value,
) -> Result<(), DispatchResponse> {
    if draft.side != Side::Sell {
        return Err(error(-4, "sell posting check applied to a buy draft"));
    }
    if draft.order_type == crate::polymarket::order::OrderType::GTD {
        return Err(error(-3, "posting GTD orders is pending expiry parity"));
    }
    if policy_check
        .get("policy_deny")
        .and_then(serde_json::Value::as_bool)
        != Some(false)
    {
        return Err(error(-3, "Polymarket policy denied; see policy_check.json"));
    }
    let deposit = deposit.to_checksum(None);
    let size = serde_json::json!(draft.size_micro);
    for (field, expected) in [
        ("status", &serde_json::json!("pass")),
        ("preflight_complete_for_posting", &serde_json::json!(true)),
        ("ctf_approved_for_all", &serde_json::json!(true)),
        ("deposit_wallet", &serde_json::json!(deposit)),
        ("token_id", &serde_json::json!(draft.token_id)),
        ("requested_size_micro", &size),
    ] {
        if preflight.get(field) != Some(expected) {
            return Err(error(
                -3,
                format!("sell preflight field '{field}' does not cover this draft; refusing"),
            ));
        }
    }
    let Some(map) = policy_check.as_object_mut() else {
        return Err(error(-4, "policy check is not an object"));
    };
    let reason = "sell can be posted after final review because chain CTF balance, CLOB conditional balance and CTF operator approval checks passed";
    for (key, value) in [
        ("status", serde_json::json!("approved")),
        ("can_post", serde_json::json!(true)),
        ("reason", serde_json::json!(reason)),
        ("approval_reason", serde_json::json!(reason)),
        ("signing_enabled", serde_json::json!(true)),
        ("posting_enabled", serde_json::json!(true)),
    ] {
        map.insert(key.into(), value);
    }
    Ok(())
}

fn venue_config_key(wallet: &str) -> String {
    format!("settings/{wallet}/venue.toml")
}

/// Read Polymarket-owned venue preferences from this Petal's state namespace.
/// Broker wallet policy is intentionally neither read nor interpreted here.
pub fn venue_config(wallet: &str) -> Result<PolymarketVenueConfig, DispatchResponse> {
    crate::polymarket::validate_wallet_name(wallet)
        .map_err(|validation_error| error(-3, validation_error.to_string()))?;
    let bytes = match petal::sdk::store_get(&venue_config_key(wallet), MAX_POLICY_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => {
            return Ok(PolymarketVenueConfig::default());
        }
        Err(error) => return Err(sdk_error(error)),
    };
    let raw = core::str::from_utf8(&bytes)
        .map_err(|e| error(-4, format!("venue configuration is not utf-8: {e}")))?;
    toml::from_str(raw).map_err(|e| error(-4, format!("venue configuration parse: {e}")))
}

pub fn read_venue_config(wallet: &str) -> DispatchResponse {
    if let Err(validation_error) = crate::polymarket::validate_wallet_name(wallet) {
        return error(-3, validation_error.to_string());
    }
    match petal::sdk::store_get(&venue_config_key(wallet), MAX_POLICY_BYTES) {
        Ok(bytes) => DispatchResponse::Read(bytes),
        Err(SdkError::Host(HostStatus::NotFound)) => {
            DispatchResponse::Read(b"enabled = false\nallow_neg_risk = true\n".to_vec())
        }
        Err(host_error) => sdk_error(host_error),
    }
}

pub fn write_venue_config(wallet: &str, body: &[u8]) -> DispatchResponse {
    if let Err(validation_error) = crate::polymarket::validate_wallet_name(wallet) {
        return error(-3, validation_error.to_string());
    }
    let raw = match core::str::from_utf8(body) {
        Ok(raw) => raw,
        Err(parse_error) => {
            return error(
                -3,
                format!("venue configuration is not utf-8: {parse_error}"),
            );
        }
    };
    if let Err(parse_error) = toml::from_str::<PolymarketVenueConfig>(raw) {
        return error(-3, format!("venue configuration parse: {parse_error}"));
    }
    match petal::sdk::store_put(&venue_config_key(wallet), body, false) {
        Ok(()) => DispatchResponse::Write,
        Err(error) => sdk_error(error),
    }
}

pub fn daily_posted_microusd(wallet: &str) -> (bool, Option<u64>) {
    let prefix = format!("trade/{wallet}/receipts/");
    let keys = match petal::sdk::store_list(&prefix, MAX_LIST_BYTES) {
        Ok(keys) => keys,
        Err(_) => return (false, None),
    };
    let cutoff = now_millis().saturating_sub(24 * 60 * 60 * 1000);
    let mut present = BTreeSet::new();
    for key in &keys {
        let rest = key.strip_prefix(&prefix).unwrap_or(key);
        let Some(id) = rest.strip_suffix("/receipt.json") else {
            continue;
        };
        present.insert(id.to_string());
    }
    let audited = match audited_receipt_ids_since(wallet, cutoff) {
        Ok(ids) => ids,
        Err(_) => return (false, None),
    };
    for id in audited {
        if !present.contains(&id) {
            return (false, None);
        }
    }
    let mut total = 0u64;
    for key in keys {
        if !key.ends_with("/receipt.json") {
            continue;
        }
        let Some(bytes) = store_get(&key) else {
            return (false, None);
        };
        let receipt: StoreTradeReceiptPolicy = match serde_json::from_slice(&bytes) {
            Ok(receipt) => receipt,
            Err(_) => return (false, None),
        };
        total = total.saturating_add(receipt_daily_exposure_microusd(&receipt, cutoff));
    }
    (true, Some(total))
}

/// What one receipt counts against `max_daily_usd` when the day began at
/// `cutoff_ms`: a BUY posted since then, unless its CLOB status excludes it,
/// counts its amount plus its fee bound. A BUY receipt written before fee
/// bounds were recorded counts the bound at the highest rate this Petal
/// accepts, since its market's rate is unknown.
pub fn receipt_daily_exposure_microusd(receipt: &StoreTradeReceiptPolicy, cutoff_ms: u128) -> u64 {
    if receipt.posted_ms < cutoff_ms || receipt.side != Side::Buy {
        return 0;
    }
    if clob_status_excluded_from_daily_cap(receipt.clob_status.as_str(), receipt.order_type) {
        return 0;
    }
    let fee_cap = receipt
        .fee_cap_microusd
        .unwrap_or_else(|| max_buy_fee_cap(receipt.amount_microusd));
    receipt.amount_microusd.saturating_add(fee_cap)
}

pub fn audited_receipt_ids_since(wallet: &str, cutoff_ms: u128) -> Result<Vec<String>, SdkError> {
    let key = format!("trade/{wallet}/audit.jsonl");
    let bytes = match petal::sdk::store_get(&key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let text = core::str::from_utf8(&bytes).map_err(|_| SdkError::Host(HostStatus::Invalid))?;
    let mut ids = Vec::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("event").and_then(serde_json::Value::as_str) != Some("receipt_written") {
            continue;
        }
        let ts = v
            .get("ts_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as u128;
        if ts < cutoff_ms {
            continue;
        }
        if let Some(id) = v
            .get("details")
            .and_then(|details| details.get("draft_id"))
            .and_then(serde_json::Value::as_str)
        {
            ids.push(id.to_string());
        }
    }
    Ok(ids)
}

pub fn verify_sell_preflight(
    wallet: &str,
    owner: Address,
    deposit: Address,
    token_id: &str,
    size_micro: u64,
    neg_risk: bool,
) -> Result<serde_json::Value, DispatchResponse> {
    let deposit_user = deposit.to_checksum(None);
    let data_api_holding_micro = get_json::<Vec<Position>>(&url_with_query(
        &format!("{}/positions", crate::runtime_config::data_url()),
        &[("user", &deposit_user)],
    ))
    .ok()
    .map(|positions| {
        positions
            .iter()
            .find(|position| position.asset == token_id)
            .and_then(position_size_micro)
            .unwrap_or(0)
    });

    let creds = load_creds(wallet)?;
    let clob_balance_allowance = clob_l2_get_json(
        owner,
        &creds,
        "/balance-allowance",
        &[
            ("asset_type", "CONDITIONAL"),
            ("token_id", token_id),
            ("signature_type", "3"),
        ],
    )?;
    let operator = if neg_risk {
        NEG_RISK_EXCHANGE
    } else {
        CTF_EXCHANGE
    };
    let chain_ctf_balance = read_chain_ctf_balance(deposit, token_id)?;
    let ctf_approved = read_chain_ctf_approval(deposit, operator)?;
    evaluate_sell_preflight(SellPreflightReads {
        deposit,
        token_id,
        size_micro,
        neg_risk,
        data_api_holding_micro,
        clob_balance_allowance,
        chain_ctf_balance,
        ctf_approved,
    })
}

/// Live reads a sell preflight judges. Every field except the Data API
/// holding is authoritative; a failed read never reaches this struct.
pub struct SellPreflightReads<'a> {
    pub deposit: Address,
    pub token_id: &'a str,
    pub size_micro: u64,
    pub neg_risk: bool,
    pub data_api_holding_micro: Option<u64>,
    pub clob_balance_allowance: serde_json::Value,
    pub chain_ctf_balance: u64,
    pub ctf_approved: bool,
}

pub fn evaluate_sell_preflight(
    reads: SellPreflightReads,
) -> Result<serde_json::Value, DispatchResponse> {
    let SellPreflightReads {
        deposit,
        token_id,
        size_micro,
        neg_risk,
        data_api_holding_micro,
        clob_balance_allowance,
        chain_ctf_balance,
        ctf_approved,
    } = reads;
    let clob_balance_micro = clob_balance_allowance
        .get("balance")
        .and_then(parse_clob_raw_micro)
        .ok_or_else(|| error(-4, "CLOB conditional balance response missing balance"))?;
    if clob_balance_micro < size_micro {
        return Err(error(
            -3,
            format!(
                "cannot sell {} shares: CLOB conditional balance reports only {}",
                format_micro(size_micro),
                format_micro(clob_balance_micro)
            ),
        ));
    }
    let operator = if neg_risk {
        NEG_RISK_EXCHANGE
    } else {
        CTF_EXCHANGE
    };
    if chain_ctf_balance < size_micro {
        return Err(error(
            -3,
            format!(
                "cannot sell {} shares: on-chain CTF balance for derived deposit wallet {} is only {}",
                format_micro(size_micro),
                deposit.to_checksum(None),
                format_micro(chain_ctf_balance)
            ),
        ));
    }
    if !ctf_approved {
        return Err(error(
            -3,
            format!(
                "cannot sell before passkey: deposit wallet {} has not approved {} for CTF tokens. Re-run onboarding to restore approvals.",
                deposit.to_checksum(None),
                operator.to_checksum(None)
            ),
        ));
    }

    Ok(serde_json::json!({
        "status": "pass",
        "source": "clob_conditional_balance_and_chain_ctf",
        "preflight_complete_for_posting": true,
        "chain_ctf_balance_checked": true,
        "ctf_approval_checked": true,
        "reason": "sell preflight passed CLOB conditional balance, on-chain CTF balance, and CTF operator approval checks; Data API holdings are included as corroborating evidence when available",
        "deposit_wallet": deposit.to_checksum(None),
        "deposit_wallet_source": "live_factory_resolved",
        "token_id": token_id,
        "requested_size_micro": size_micro,
        "requested_size": format_micro(size_micro),
        "data_api_holding_checked": data_api_holding_micro.is_some(),
        "data_api_holding_micro": data_api_holding_micro,
        "data_api_holding": data_api_holding_micro.map(format_micro),
        "clob_balance_micro": clob_balance_micro,
        "clob_balance": format_micro(clob_balance_micro),
        "clob_balance_allowance": clob_balance_allowance,
        "chain_ctf_contract": CTF.to_checksum(None),
        "chain_ctf_balance_micro": chain_ctf_balance,
        "chain_ctf_balance": format_micro(chain_ctf_balance),
        "ctf_operator": operator.to_checksum(None),
        "ctf_operator_kind": if neg_risk { "neg_risk_exchange" } else { "ctf_exchange" },
        "ctf_approved_for_all": ctf_approved,
        "signing_enabled": true,
        "posting_enabled": true
    }))
}

pub fn position_size_micro(position: &Position) -> Option<u64> {
    position
        .size
        .and_then(|size| parse_json_f64_micro(size).ok())
}

pub fn parse_clob_raw_micro(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::String(s) => s.trim().parse::<u64>().ok(),
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Some(u)
            } else {
                n.as_f64().and_then(|f| parse_json_f64_micro(f).ok())
            }
        }
        _ => None,
    }
}

pub fn parse_json_f64_micro(value: f64) -> Result<u64, DispatchResponse> {
    if !value.is_finite() || value < 0.0 {
        return Err(error(-4, "decimal value is not a non-negative number"));
    }
    parse_micro(&format!("{value}")).map_err(|e| error(-4, e.to_string()))
}

pub fn evaluate_local_polymarket_order(
    policy: &PolymarketVenueConfig,
    ctx: &LocalPolymarketOrderCtx,
) -> Vec<LocalPolicyCheck> {
    let mut out = Vec::new();
    if !policy.enabled {
        out.push(local_policy_check(
            "enabled",
            LocalPolicyOutcome::Deny,
            "Polymarket trading is disabled for this wallet; set enabled = true in this Petal's venue.toml to opt in",
        ));
    } else {
        out.push(local_policy_check(
            "enabled",
            LocalPolicyOutcome::Pass,
            "trading enabled",
        ));
    }

    if ctx.closed || !ctx.active {
        out.push(local_policy_check(
            "market",
            LocalPolicyOutcome::Deny,
            format!(
                "market is not tradable (active={}, closed={})",
                ctx.active, ctx.closed
            ),
        ));
    } else if !ctx.order_book_enabled {
        out.push(local_policy_check(
            "market",
            LocalPolicyOutcome::Deny,
            "market has no order book enabled",
        ));
    } else if !ctx.binary_outcomes {
        out.push(local_policy_check(
            "market",
            LocalPolicyOutcome::Deny,
            "market is malformed or not a binary YES/NO market",
        ));
    } else {
        out.push(local_policy_check(
            "market",
            LocalPolicyOutcome::Pass,
            "active, order book enabled, binary outcomes",
        ));
    }

    out.push(local_policy_list_check(
        "slug",
        &ctx.slug,
        &policy.allowed_slugs,
        &policy.denied_slugs,
    ));
    out.push(local_policy_list_check(
        "condition_id",
        &ctx.condition_id,
        &policy.allowed_condition_ids,
        &policy.denied_condition_ids,
    ));

    if ctx.neg_risk && !policy.allow_neg_risk {
        out.push(local_policy_check(
            "neg_risk",
            LocalPolicyOutcome::Deny,
            "neg-risk markets are disabled by policy (allow_neg_risk = false)",
        ));
    } else {
        out.push(local_policy_check(
            "neg_risk",
            LocalPolicyOutcome::Pass,
            format!("neg_risk={} permitted", ctx.neg_risk),
        ));
    }

    if ctx.side == LocalPolicySide::Sell {
        out.push(local_policy_check(
            "caps",
            LocalPolicyOutcome::Pass,
            "sell orders are risk-reducing; USD caps not applied",
        ));
        return out;
    }

    // A BUY can cost its amount plus the most Polymarket can charge in fees,
    // and every USD threshold below applies to that whole cost.
    let order_usd = ctx.amount_microusd.saturating_add(ctx.fee_cap_microusd);
    let order_text = format!(
        "{} (amount {} + fee bound {})",
        format_micro(order_usd),
        format_micro(ctx.amount_microusd),
        format_micro(ctx.fee_cap_microusd)
    );

    if let Some(cap) = policy.max_order_usd {
        if order_usd > cap {
            out.push(local_policy_check(
                "max_order_usd",
                LocalPolicyOutcome::Deny,
                format!(
                    "order {order_text} USD exceeds max_order_usd {}",
                    format_micro(cap)
                ),
            ));
        } else {
            out.push(local_policy_check(
                "max_order_usd",
                LocalPolicyOutcome::Pass,
                format!("{order_text} <= {}", format_micro(cap)),
            ));
        }
    }

    if let Some(cap) = policy.max_daily_usd {
        match (ctx.receipt_store_readable, ctx.daily_posted_microusd) {
            (false, _) | (_, None) => out.push(local_policy_check(
                "max_daily_usd",
                LocalPolicyOutcome::Deny,
                "daily cap configured but posted exposure is unknown (receipt store unreadable) - refusing rather than trading uncapped",
            )),
            (true, Some(daily)) => {
                let total = daily.saturating_add(order_usd);
                if total > cap {
                    out.push(local_policy_check(
                        "max_daily_usd",
                        LocalPolicyOutcome::Deny,
                        format!(
                            "posted {} USD + order {order_text} USD exceeds max_daily_usd {}",
                            format_micro(daily),
                            format_micro(cap)
                        ),
                    ));
                } else {
                    out.push(local_policy_check(
                        "max_daily_usd",
                        LocalPolicyOutcome::Pass,
                        format!(
                            "{} + {order_text} <= {}",
                            format_micro(daily),
                            format_micro(cap)
                        ),
                    ));
                }
            }
        }
    }

    if let Some(maxp) = policy.max_price {
        if ctx.limit_price_micro > maxp {
            out.push(local_policy_check(
                "max_price",
                LocalPolicyOutcome::Deny,
                format!(
                    "limit price {} exceeds policy max_price {}",
                    format_micro(ctx.limit_price_micro),
                    format_micro(maxp)
                ),
            ));
        } else {
            out.push(local_policy_check(
                "max_price",
                LocalPolicyOutcome::Pass,
                format!(
                    "{} <= {}",
                    format_micro(ctx.limit_price_micro),
                    format_micro(maxp)
                ),
            ));
        }
    }

    if let Some(threshold) = policy.require_flag_above_usd
        && order_usd > threshold
    {
        out.push(local_policy_check(
            "require_flag_above_usd",
            LocalPolicyOutcome::Warn,
            format!(
                "order {order_text} USD is above {} - acknowledge before value-moving post",
                format_micro(threshold)
            ),
        ));
    }

    out
}

pub fn local_policy_check(
    rule: &str,
    outcome: LocalPolicyOutcome,
    message: impl Into<String>,
) -> LocalPolicyCheck {
    LocalPolicyCheck {
        rule: format!("polymarket.{rule}"),
        outcome,
        message: message.into(),
    }
}

pub fn local_policy_list_check(
    name: &str,
    value: &str,
    allowed: &BTreeSet<String>,
    denied: &BTreeSet<String>,
) -> LocalPolicyCheck {
    if denied.contains(value) {
        local_policy_check(
            name,
            LocalPolicyOutcome::Deny,
            format!("'{value}' is denylisted"),
        )
    } else if !allowed.is_empty() && !allowed.contains(value) {
        local_policy_check(
            name,
            LocalPolicyOutcome::Deny,
            format!("'{value}' is not on the allowlist (allowlist-only mode)"),
        )
    } else {
        local_policy_check(
            name,
            LocalPolicyOutcome::Pass,
            format!("'{value}' permitted"),
        )
    }
}

pub fn local_policy_has_deny(checks: &[LocalPolicyCheck]) -> bool {
    checks
        .iter()
        .any(|check| check.outcome == LocalPolicyOutcome::Deny)
}

pub fn local_policy_has_warn(checks: &[LocalPolicyCheck]) -> bool {
    checks
        .iter()
        .any(|check| check.outcome == LocalPolicyOutcome::Warn)
}

pub fn parse_api_float_micro(value: f64, field: &str) -> Result<u64, DispatchResponse> {
    if !value.is_finite() || value < 0.0 {
        return Err(error(-4, format!("{field} is not a non-negative number")));
    }
    parse_micro(&format!("{value:.6}")).map_err(|e| error(-4, e.to_string()))
}
