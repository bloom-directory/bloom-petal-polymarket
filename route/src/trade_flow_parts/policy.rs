use crate::*;
use std::collections::BTreeSet;

use crate::bloom_petal_sdk::{DispatchResponse, HostStatus, SdkError};
use crate::eip712::{CTF, CTF_EXCHANGE_V2, NEG_RISK_EXCHANGE_V2};
use crate::order::{format_micro, parse_micro};
use crate::polymarket::{Position, Result, Side};
use alloy::primitives::Address;

pub(crate) fn enable_trade_posting(policy_check: &mut serde_json::Value, reason: &str) {
    if let Some(map) = policy_check.as_object_mut() {
        map.insert("status".into(), serde_json::json!("approved"));
        map.insert("can_post".into(), serde_json::json!(true));
        map.insert("approval_reason".into(), serde_json::json!(reason));
    }
}

pub(crate) fn wallet_policy(wallet: &str) -> Result<LocalWalletPolicy, DispatchResponse> {
    let bytes =
        bloom_petal_sdk::vfs_read(&format!("wallets/{wallet}/policy.toml"), MAX_POLICY_BYTES)
            .map_err(sdk_error)?;
    let raw = core::str::from_utf8(&bytes)
        .map_err(|e| error(-4, format!("wallet policy is not utf-8: {e}")))?;
    toml::from_str(raw).map_err(|e| error(-4, format!("wallet policy parse: {e}")))
}

pub(crate) fn daily_posted_microusd(wallet: &str) -> (bool, Option<u64>) {
    let prefix = format!("trade/{wallet}/receipts/");
    let keys = match bloom_petal_sdk::store_list(&prefix, MAX_LIST_BYTES) {
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
        if receipt.posted_ms < cutoff || receipt.side != Side::Buy {
            continue;
        }
        if clob_status_excluded_from_daily_cap(receipt.clob_status.as_str(), receipt.order_type) {
            continue;
        }
        total = total.saturating_add(receipt.amount_microusd);
    }
    (true, Some(total))
}

pub(crate) fn audited_receipt_ids_since(
    wallet: &str,
    cutoff_ms: u128,
) -> Result<Vec<String>, SdkError> {
    let key = format!("trade/{wallet}/audit.jsonl");
    let bytes = match bloom_petal_sdk::store_get(&key, MAX_STORE_BYTES) {
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

pub(crate) fn verify_sell_preflight(
    wallet: &str,
    owner: Address,
    deposit: Address,
    token_id: &str,
    size_micro: u64,
    neg_risk: bool,
) -> Result<serde_json::Value, DispatchResponse> {
    let deposit_user = deposit.to_checksum(None);
    let data_api_holding_micro = get_json::<Vec<Position>>(&url_with_query(
        &format!("{DATA}/positions"),
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
        NEG_RISK_EXCHANGE_V2
    } else {
        CTF_EXCHANGE_V2
    };
    let chain_ctf_balance = read_chain_ctf_balance(deposit, token_id)?;
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
    let ctf_approved = read_chain_ctf_approval(deposit, operator)?;
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
        "ctf_operator_kind": if neg_risk { "neg_risk_exchange_v2" } else { "ctf_exchange_v2" },
        "ctf_approved_for_all": ctf_approved,
        "signing_enabled": true,
        "posting_enabled": true
    }))
}

pub(crate) fn position_size_micro(position: &Position) -> Option<u64> {
    position
        .size
        .and_then(|size| parse_json_f64_micro(size).ok())
}

pub(crate) fn parse_clob_raw_micro(value: &serde_json::Value) -> Option<u64> {
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

pub(crate) fn parse_json_f64_micro(value: f64) -> Result<u64, DispatchResponse> {
    if !value.is_finite() || value < 0.0 {
        return Err(error(-4, "decimal value is not a non-negative number"));
    }
    parse_micro(&format!("{value}")).map_err(|e| error(-4, e.to_string()))
}

pub(crate) fn evaluate_local_polymarket_order(
    policy: &LocalPolymarketPolicy,
    ctx: &LocalPolymarketOrderCtx,
) -> Vec<LocalPolicyCheck> {
    let mut out = Vec::new();
    if !policy.enabled {
        out.push(local_policy_check(
            "enabled",
            LocalPolicyOutcome::Deny,
            "Polymarket trading is disabled for this wallet; set [polymarket] enabled = true in the wallet policy to opt in",
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

    if let Some(cap) = policy.max_order_usd {
        if ctx.amount_microusd > cap {
            out.push(local_policy_check(
                "max_order_usd",
                LocalPolicyOutcome::Deny,
                format!(
                    "order {} USD exceeds max_order_usd {}",
                    format_micro(ctx.amount_microusd),
                    format_micro(cap)
                ),
            ));
        } else {
            out.push(local_policy_check(
                "max_order_usd",
                LocalPolicyOutcome::Pass,
                format!(
                    "{} <= {}",
                    format_micro(ctx.amount_microusd),
                    format_micro(cap)
                ),
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
                let total = daily.saturating_add(ctx.amount_microusd);
                if total > cap {
                    out.push(local_policy_check(
                        "max_daily_usd",
                        LocalPolicyOutcome::Deny,
                        format!(
                            "posted {} USD + order {} USD exceeds max_daily_usd {}",
                            format_micro(daily),
                            format_micro(ctx.amount_microusd),
                            format_micro(cap)
                        ),
                    ));
                } else {
                    out.push(local_policy_check(
                        "max_daily_usd",
                        LocalPolicyOutcome::Pass,
                        format!(
                            "{} + {} <= {}",
                            format_micro(daily),
                            format_micro(ctx.amount_microusd),
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
        && ctx.amount_microusd > threshold
    {
        out.push(local_policy_check(
            "require_flag_above_usd",
            LocalPolicyOutcome::Warn,
            format!(
                "order {} USD is above {} - acknowledge before value-moving post",
                format_micro(ctx.amount_microusd),
                format_micro(threshold)
            ),
        ));
    }

    out
}

pub(crate) fn local_policy_check(
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

pub(crate) fn local_policy_list_check(
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

pub(crate) fn local_policy_has_deny(checks: &[LocalPolicyCheck]) -> bool {
    checks
        .iter()
        .any(|check| check.outcome == LocalPolicyOutcome::Deny)
}

pub(crate) fn local_policy_has_warn(checks: &[LocalPolicyCheck]) -> bool {
    checks
        .iter()
        .any(|check| check.outcome == LocalPolicyOutcome::Warn)
}

pub(crate) fn parse_api_float_micro(value: f64, field: &str) -> Result<u64, DispatchResponse> {
    if !value.is_finite() || value < 0.0 {
        return Err(error(-4, format!("{field} is not a non-negative number")));
    }
    parse_micro(&format!("{value:.6}")).map_err(|e| error(-4, e.to_string()))
}
