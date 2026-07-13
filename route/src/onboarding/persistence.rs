use crate::prelude::*;

use alloy::primitives::Address;
use petal::sdk::DispatchResponse;

#[derive(Default)]
pub struct OnboardStatusExtra {
    pub stage: Option<&'static str>,
    pub deploy_tx_id: Option<String>,
    pub approve_tx_id: Option<String>,
    pub in_flight_deadline_ms: Option<u128>,
    pub relayer_auth: Option<&'static str>,
    pub last_error: Option<String>,
}

pub fn persist_onboard_status(
    wallet: &str,
    owner: Address,
    deposit: Address,
    creds_present: bool,
    extra: OnboardStatusExtra,
) -> Result<serde_json::Value, DispatchResponse> {
    let mut status = refreshed_live_onboard_status(wallet, owner, deposit, creds_present)?;
    if let Some(obj) = status.as_object_mut() {
        if let Some(stage) = extra.stage {
            obj.insert("stage".into(), serde_json::Value::String(stage.into()));
            obj.insert(
                "tradeable".into(),
                serde_json::Value::Bool(stage == "complete"),
            );
        }
        obj.insert(
            "deploy_tx_id".into(),
            extra
                .deploy_tx_id
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "approve_tx_id".into(),
            extra
                .approve_tx_id
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "in_flight_deadline_ms".into(),
            extra
                .in_flight_deadline_ms
                .map(|v| serde_json::Value::String(v.to_string()))
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "relayer_auth".into(),
            extra
                .relayer_auth
                .map(|v| serde_json::Value::String(v.into()))
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "last_error".into(),
            extra
                .last_error
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null),
        );
        obj.insert(
            "status_updated_ms".into(),
            serde_json::Value::String(now_millis().to_string()),
        );
    }
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("onboard/{wallet}/status.json"), &status, false)
    {
        return Err(error(-4, "failed to persist onboarding status"));
    }
    Ok(status)
}

pub fn persist_onboard_failure(
    wallet: &str,
    owner: Address,
    deposit: Address,
    resp: &DispatchResponse,
) -> Result<serde_json::Value, DispatchResponse> {
    let existing = stored_status_for_wallet(wallet, owner).ok();
    let string_field = |name: &str| {
        existing
            .as_ref()
            .and_then(|status| status.get(name))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let stage = existing
        .as_ref()
        .and_then(|status| status.get("stage"))
        .and_then(serde_json::Value::as_str)
        .and_then(|stage| match stage {
            "deploy" => Some("deploy"),
            "fund" => Some("fund"),
            "approve" => Some("approve"),
            "sync" => Some("sync"),
            _ => None,
        });
    persist_onboard_status(
        wallet,
        owner,
        deposit,
        store_get(&format!("creds/{wallet}/clob.json")).is_some(),
        OnboardStatusExtra {
            stage,
            deploy_tx_id: string_field("deploy_tx_id"),
            approve_tx_id: string_field("approve_tx_id"),
            in_flight_deadline_ms: string_field("in_flight_deadline_ms")
                .and_then(|value| value.parse().ok()),
            last_error: Some(dispatch_error_message(resp)),
            ..OnboardStatusExtra::default()
        },
    )
}
