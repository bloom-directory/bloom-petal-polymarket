use alloy::primitives::Address;
use serde::Deserialize;

use crate::polymarket::builder_creds::BuilderApiKeyInfo;
use crate::polymarket::{Position, validate_wallet_name};
use crate::prelude::*;
use petal::sdk::DispatchResponse;

pub fn status(wallet: &str, account: &petal::WalletAccount) -> DispatchResponse {
    let legacy_eoa = match crate::relayer_config::load_relayer_config() {
        Ok(config) => config.legacy_eoa_mode,
        Err(resp) => return resp,
    };
    let (owner, status) = match wallet_status(wallet, account) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let tradeable = !legacy_eoa
        && status
            .get("tradeable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "owner_address": owner.to_checksum(None),
        "mode": if legacy_eoa { "legacy_eoa_credentials_read_only" } else { "deposit_wallet" },
        "deposit_wallet": status.get("deposit_wallet").cloned().unwrap_or(serde_json::Value::Null),
        "tradeable": tradeable,
        "onboarding_stage": status.get("stage").cloned().unwrap_or(serde_json::json!("not_started")),
        "credentials_present": status.get("creds_present").cloned().unwrap_or(serde_json::json!(false)),
        "next_required_action": if legacy_eoa || tradeable { None } else { Some("continue_onboarding") },
    }))
}

pub fn obligations(wallet: &str, account: &petal::WalletAccount) -> DispatchResponse {
    let legacy_eoa = match crate::relayer_config::load_relayer_config() {
        Ok(config) => config.legacy_eoa_mode,
        Err(resp) => return resp,
    };
    let (owner, status) = match wallet_status(wallet, account) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let subject = if legacy_eoa {
        owner
    } else {
        let Some(deposit) = fundable_deposit_wallet_from_status(&status) else {
            return error(
                -3,
                "deposit wallet is not factory-resolved; begin onboarding first",
            );
        };
        deposit
    };
    let positions = match get_json::<Vec<Position>>(&url_with_query(
        &format!("{}/positions", crate::runtime_config::data_url()),
        &[("user", &subject.to_checksum(None))],
    )) {
        Ok(positions) => positions,
        Err(resp) => return resp,
    };
    let open: Vec<_> = positions
        .into_iter()
        .filter(|position| position.size.unwrap_or(0.0) > 0.0)
        .map(|position| serde_json::json!({
            "title": position.title,
            "outcome": position.outcome,
            "token_id": position.asset,
            "condition_id": position.condition_id,
            "size": position.size,
            "avg_price": position.avg_price,
            "current_price": position.cur_price,
            "redeemable": position.redeemable,
            "next_action": if legacy_eoa { "read_only" } else if position.redeemable { "redeem" } else { "sell_to_close_or_wait_for_redemption" },
        }))
        .collect();
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "account_address": subject.to_checksum(None),
        "deposit_wallet": if legacy_eoa { serde_json::Value::Null } else { serde_json::Value::String(subject.to_checksum(None)) },
        "credentials_read_only": legacy_eoa,
        "open_positions": open,
        "next_required_action": if open.is_empty() { "none" } else { "review_open_positions" },
    }))
}

pub fn builder_keys(wallet: &str, account: &petal::WalletAccount) -> DispatchResponse {
    if let Err(err) = validate_wallet_name(wallet) {
        return error(-3, err.to_string());
    }
    let owner = match wallet_address(account) {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let value = match clob_l2_get_json(owner, &creds, "/auth/builder-api-key", &[]) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let stored_key = match load_builder_credentials(wallet) {
        Ok(stored) => stored.map(|credentials| credentials.key),
        Err(resp) => return resp,
    };
    let entries = value
        .as_array()
        .or_else(|| value.get("data").and_then(serde_json::Value::as_array))
        .or_else(|| value.get("keys").and_then(serde_json::Value::as_array));
    let keys: Vec<_> = entries
        .into_iter()
        .flatten()
        .filter_map(BuilderApiKeyInfo::from_value)
        .filter(|key| !key.key.trim().is_empty())
        .map(|key| {
            serde_json::json!({
                "key": key.key,
                "created_at": key.created_at,
                "revoked_at": key.revoked_at,
                "stored_by_petal": stored_key.as_deref() == Some(key.key.as_str()),
            })
        })
        .collect();
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "keys": keys,
        "secrets_exposed": false,
    }))
}

#[derive(Deserialize)]
struct RevokeBuilderKey {
    confirm: bool,
    #[serde(default)]
    key: Option<String>,
}

pub fn revoke_builder_key(
    wallet: &str,
    account: &petal::WalletAccount,
    body: &[u8],
) -> DispatchResponse {
    if let Err(err) = validate_wallet_name(wallet) {
        return error(-3, err.to_string());
    }
    let text = match core::str::from_utf8(body) {
        Ok(text) => text.trim(),
        Err(_) => return error(-3, "builder-key revoke body must be UTF-8"),
    };
    let key = if matches!(text.to_ascii_lowercase().as_str(), "confirm" | "y" | "yes") {
        None
    } else {
        let request: RevokeBuilderKey = match serde_json::from_str(text) {
            Ok(request) => request,
            Err(err) => return error(-3, format!("builder-key revoke JSON: {err}")),
        };
        if !request.confirm {
            return error(-3, "builder-key revoke must set confirm=true");
        }
        if request
            .key
            .as_deref()
            .is_some_and(|key| !petal::is_safe_segment(key))
        {
            return error(-3, "invalid builder-key id");
        }
        request.key
    };
    let owner = match wallet_address(account) {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let stored = match load_builder_credentials(wallet) {
        Ok(stored) => stored,
        Err(resp) => return resp,
    };
    let request_body = key
        .as_deref()
        .map(|key| serde_json::json!({"key": key}).to_string())
        .unwrap_or_default();
    if let Err(resp) = clob_l2_delete_json(owner, &creds, "/auth/builder-api-key", &request_body) {
        return resp;
    }
    if stored
        .as_ref()
        .is_some_and(|stored| key.is_none() || key.as_deref() == Some(stored.key.as_str()))
        && let Err(resp) = delete_builder_credentials(wallet)
    {
        return resp;
    }
    DispatchResponse::Write
}

pub fn wallet_status(
    wallet: &str,
    account: &petal::WalletAccount,
) -> Result<(Address, serde_json::Value), DispatchResponse> {
    validate_wallet_name(wallet).map_err(polymarket_error)?;
    let owner = wallet_address(account)?;
    Ok((owner, local_status_for_wallet(wallet, owner)?))
}
