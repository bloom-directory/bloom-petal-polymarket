use alloy::primitives::Address;
use serde::Deserialize;

use crate::polymarket::builder_creds::BuilderApiKeyInfo;
use crate::polymarket::{Position, validate_wallet_name};
use crate::prelude::*;
use petal::sdk::{DispatchResponse, HostStatus, SdkError};

pub(crate) fn status(wallet: &str) -> DispatchResponse {
    let (owner, status) = match wallet_status(wallet) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let tradeable = status
        .get("tradeable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "owner_address": owner.to_checksum(None),
        "mode": "deposit_wallet",
        "deposit_wallet": status.get("deposit_wallet").cloned().unwrap_or(serde_json::Value::Null),
        "tradeable": tradeable,
        "onboarding_stage": status.get("stage").cloned().unwrap_or(serde_json::json!("not_started")),
        "credentials_present": status.get("creds_present").cloned().unwrap_or(serde_json::json!(false)),
        "next_required_action": if tradeable { None } else { Some("continue_onboarding") },
    }))
}

pub(crate) fn buying_power(wallet: &str) -> DispatchResponse {
    let (owner, status) = match wallet_status(wallet) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let balance_allowance = match clob_l2_get_json(
        owner,
        &creds,
        "/balance-allowance",
        &[("asset_type", "COLLATERAL"), ("signature_type", "3")],
    ) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let has_balance = balance_allowance
        .get("balance")
        .and_then(parse_json_u256)
        .is_some_and(|balance| !balance.is_zero());
    let tradeable = status
        .get("tradeable")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "spendable": {
            "asset": "pUSD",
            "raw": balance_allowance.get("balance").cloned().unwrap_or(serde_json::Value::Null),
            "source": "clob_balance_allowance",
            "clob_balance_allowance": balance_allowance,
        },
        "can_trade_now": tradeable && has_balance,
        "funding_needed": !has_balance,
        "funding_options_ref": format!("account/{wallet}/funding_options.json"),
    }))
}

pub(crate) fn funding_options(wallet: &str) -> DispatchResponse {
    if let Err(err) = validate_wallet_name(wallet) {
        return error(-3, err.to_string());
    }
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "target_asset": "pUSD",
        "options": [{
            "from": "pUSD",
            "supported": true,
            "review_required": true,
            "fund_route": format!("fund/{wallet}/new"),
            "execution": "generic_evm_outbox_direct_erc20_transfer",
        }, {
            "from": "native_or_other_erc20",
            "supported": true,
            "review_required": true,
            "fund_route": format!("fund/{wallet}/new"),
            "execution": "enso_quote_then_generic_evm_outbox",
            "enso_key_configured": load_enso_api_key().is_ok(),
            "enso_router_configured": load_enso_router().is_ok(),
        }],
    }))
}

pub(crate) fn obligations(wallet: &str) -> DispatchResponse {
    let (_, status) = match wallet_status(wallet) {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let Some(deposit) = fundable_deposit_wallet_from_status(&status) else {
        return error(
            -3,
            "deposit wallet is not factory-resolved; begin onboarding first",
        );
    };
    let positions = match get_json::<Vec<Position>>(&url_with_query(
        &format!("{DATA}/positions"),
        &[("user", &deposit.to_checksum(None))],
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
            "next_action": if position.redeemable { "redeem" } else { "sell_to_close_or_wait_for_redemption" },
        }))
        .collect();
    petal::read_json_value(&serde_json::json!({
        "wallet": wallet,
        "deposit_wallet": deposit.to_checksum(None),
        "open_positions": open,
        "next_required_action": if open.is_empty() { "none" } else { "review_open_positions" },
    }))
}

pub(crate) fn builder_keys(wallet: &str) -> DispatchResponse {
    if let Err(err) = validate_wallet_name(wallet) {
        return error(-3, err.to_string());
    }
    let owner = match wallet_address(wallet) {
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

pub(crate) fn revoke_builder_key(wallet: &str, body: &[u8]) -> DispatchResponse {
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
    let owner = match wallet_address(wallet) {
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

#[derive(Deserialize)]
struct EnsoSettingsInput {
    api_key: String,
    router: String,
}

pub(crate) fn write_enso_api_key(body: &[u8]) -> DispatchResponse {
    let text = match core::str::from_utf8(body) {
        Ok(value) => value.trim(),
        Err(_) => return error(-3, "Enso API key must be UTF-8"),
    };
    let (key, router) = if text.starts_with('{') {
        let input: EnsoSettingsInput = match serde_json::from_str(text) {
            Ok(input) => input,
            Err(err) => return error(-3, format!("Enso settings JSON: {err}")),
        };
        let router = match input.router.parse::<Address>() {
            Ok(router) if router != Address::ZERO => router,
            Ok(_) => return error(-3, "Enso router cannot be zero"),
            Err(err) => return error(-3, format!("Enso router address: {err}")),
        };
        (input.api_key, Some(router))
    } else {
        (text.to_string(), None)
    };
    if key.is_empty() || key.len() > 4096 || key.chars().any(char::is_whitespace) {
        return error(-3, "Enso API key must be 1-4096 non-whitespace characters");
    }
    if let Some(router) = router
        && let Err(err) = petal::sdk::store_put(
            "settings/enso-router",
            router.to_checksum(None).as_bytes(),
            false,
        )
    {
        return sdk_error(err);
    }
    match petal::sdk::store_put("creds/enso-api-key", key.as_bytes(), true) {
        Ok(()) => DispatchResponse::Write,
        Err(err) => sdk_error(err),
    }
}

pub(crate) fn load_enso_router() -> Result<Address, DispatchResponse> {
    let bytes = petal::sdk::store_get("settings/enso-router", 128).map_err(|err| match err {
        SdkError::Host(HostStatus::NotFound) => error(
            -3,
            "trusted Enso router is not configured; write {\"api_key\":\"...\",\"router\":\"0x...\"} to settings/enso-api-key",
        ),
        other => sdk_error(other),
    })?;
    let raw = core::str::from_utf8(&bytes)
        .map_err(|_| error(-4, "stored Enso router is not valid UTF-8"))?;
    raw.parse::<Address>()
        .map_err(|err| error(-4, format!("stored Enso router address: {err}")))
}

pub(crate) fn load_enso_api_key() -> Result<String, DispatchResponse> {
    let bytes = petal::sdk::store_get("creds/enso-api-key", 4096).map_err(|err| match err {
        SdkError::Host(HostStatus::NotFound) => error(
            -3,
            "Enso API key is not configured; write it to settings/enso-api-key",
        ),
        other => sdk_error(other),
    })?;
    String::from_utf8(bytes).map_err(|_| error(-4, "stored Enso API key is not valid UTF-8"))
}

fn wallet_status(wallet: &str) -> Result<(Address, serde_json::Value), DispatchResponse> {
    validate_wallet_name(wallet).map_err(polymarket_error)?;
    let owner = wallet_address(wallet)?;
    Ok((owner, local_status_for_wallet(wallet, owner)?))
}
