use crate::prelude::*;

use crate::polymarket::eip712::PUSD;
use crate::polymarket::{Result, derive_deposit_wallet_address};
use alloy::primitives::{Address, U256};
use petal::sdk::DispatchResponse;

pub fn local_onboard_status(
    wallet: &str,
    owner: Address,
    stage: &str,
    running: bool,
    creds_present: bool,
    message: &str,
) -> serde_json::Value {
    let chain_id = crate::runtime_config::configured_chain()
        .map(|(_, id)| id)
        .unwrap_or(137);
    let deposit = derive_deposit_wallet_address(&owner, chain_id);
    serde_json::json!({
        "wallet": wallet,
        "owner": format!("{owner:#x}"),
        "stage": stage,
        "running": running,
        "tradeable": false,
        "creds_present": creds_present,
        "deposit_wallet": {
            "address": deposit.to_checksum(None),
            "source": "local_estimate_unverified",
            "fundable": false,
            "warning": "do not fund this local estimate; full onboarding must resolve the live factory address first"
        },
        "approvals": {
            "required": true
        },
        "message": message
    })
}

pub struct LiveOnboardStatus<'a> {
    pub wallet: &'a str,
    pub owner: Address,
    pub deposit: Address,
    pub stage: &'a str,
    pub running: bool,
    pub creds_present: bool,
    pub tradeable: bool,
    pub message: &'a str,
    pub probes: serde_json::Value,
}

pub fn local_onboard_status_with_live_deposit(status: LiveOnboardStatus<'_>) -> serde_json::Value {
    serde_json::json!({
        "wallet": status.wallet,
        "owner": status.owner.to_checksum(None),
        "stage": status.stage,
        "running": status.running,
        "tradeable": status.tradeable,
        "creds_present": status.creds_present,
        "deposit_wallet": {
            "address": status.deposit.to_checksum(None),
            "source": "live_factory_resolved",
            "fundable": true,
            "warning": serde_json::Value::Null
        },
        "approvals": {
            "required": true
        },
        "probes": status.probes,
        "message": status.message
    })
}

pub fn refreshed_live_onboard_status(
    wallet: &str,
    owner: Address,
    deposit: Address,
    creds_present: bool,
) -> Result<serde_json::Value, DispatchResponse> {
    let deployed = read_chain_deposit_wallet_deployed(deposit)?;
    let pusd_balance = if deployed {
        read_chain_erc20_balance(PUSD, deposit)?
    } else {
        U256::ZERO
    };
    let approvals_in_place = if deployed && !pusd_balance.is_zero() {
        read_chain_approvals(deposit)?
    } else {
        false
    };
    let (clob_synced, clob_balance, clob_allowance) =
        if deployed && !pusd_balance.is_zero() && approvals_in_place && creds_present {
            let creds = load_creds(wallet)?;
            read_clob_collateral_sync(owner, &creds)?
        } else {
            (false, None, None)
        };

    let (stage, tradeable, message) = if !deployed {
        (
            "deploy",
            false,
            "deposit wallet resolved from the live factory; waiting for the native relayer deploy stage",
        )
    } else if pusd_balance.is_zero() {
        (
            "fund",
            false,
            "deposit wallet is deployed; waiting for pUSD funding",
        )
    } else if !approvals_in_place {
        (
            "approve",
            false,
            "deposit wallet holds pUSD; waiting for exchange and adapter approvals",
        )
    } else if !creds_present {
        (
            "creds",
            false,
            "deposit wallet is funded and approved; write begin to mint or derive CLOB credentials",
        )
    } else if !clob_synced {
        (
            "sync",
            false,
            "deposit wallet is funded and approved; waiting for CLOB collateral balance/allowance sync",
        )
    } else {
        (
            "complete",
            true,
            "local read-only probes show the deposit wallet is deployed, funded, approved, credentialed, and CLOB-synced",
        )
    };

    Ok(local_onboard_status_with_live_deposit(LiveOnboardStatus {
        wallet,
        owner,
        deposit,
        stage,
        running: false,
        creds_present,
        tradeable,
        message,
        probes: serde_json::json!({
            "source": "vfs_chain_and_clob_read_only",
            "deposit_wallet_deployed": deployed,
            "pusd_balance_raw": pusd_balance.to_string(),
            "approvals_in_place": approvals_in_place,
            "clob_collateral_balance_raw": clob_balance.map(|v| v.to_string()),
            "clob_collateral_allowance_raw": clob_allowance.map(|v| v.to_string()),
            "clob_collateral_synced": clob_synced
        }),
    }))
}

pub fn local_status_for_wallet(
    wallet: &str,
    owner: Address,
) -> Result<serde_json::Value, DispatchResponse> {
    let mut status = stored_status_for_wallet(wallet, owner)?;
    let creds_present = store_get(&format!("creds/{wallet}/clob.json")).is_some();
    status["creds_present"] = serde_json::json!(creds_present);
    if creds_present && crate::relayer_config::load_relayer_config()?.legacy_eoa_mode {
        return Ok(crate::relayer_config::legacy_eoa_status(wallet, owner));
    }
    let deposit_value = status.get("deposit_wallet");
    let source = deposit_value
        .and_then(|value| value.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if source != "live_factory_resolved" {
        return Ok(status);
    }
    let deposit = deposit_value
        .and_then(|value| value.get("address"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, "stored onboarding status is missing deposit wallet"))?
        .parse::<Address>()
        .map_err(|e| error(-4, format!("stored deposit wallet parse: {e}")))?;
    let mut refreshed = refreshed_live_onboard_status(wallet, owner, deposit, creds_present)?;
    preserve_onboard_metadata(&status, &mut refreshed);
    if refreshed != status
        && let DispatchResponse::Error { .. } =
            store_put_json(&format!("onboard/{wallet}/status.json"), &refreshed, false)
    {
        return Err(error(-4, "failed to refresh onboarding status"));
    }
    Ok(refreshed)
}

pub fn preserve_onboard_metadata(previous: &serde_json::Value, refreshed: &mut serde_json::Value) {
    let refreshed_complete =
        refreshed.get("stage").and_then(serde_json::Value::as_str) == Some("complete");
    let Some(obj) = refreshed.as_object_mut() else {
        return;
    };
    for key in [
        "deploy_tx_id",
        "approve_tx_id",
        "relayer_auth",
        "status_updated_ms",
    ] {
        if let Some(value) = previous.get(key) {
            obj.insert(key.into(), value.clone());
        }
    }
    if previous
        .get("in_flight_deadline_ms")
        .is_some_and(|value| !value.is_null())
        && !refreshed_complete
    {
        obj.insert(
            "in_flight_deadline_ms".into(),
            previous["in_flight_deadline_ms"].clone(),
        );
    }
    if previous
        .get("last_error")
        .is_some_and(|value| !value.is_null())
        && !refreshed_complete
    {
        obj.insert("last_error".into(), previous["last_error"].clone());
    }
}

/// Project guidance from the current account state; never read a second hint store.
pub fn project_next_action(
    status: &mut serde_json::Value,
    wallet: &str,
    account: u32,
    approval_pending: bool,
    login_retry_after_ms: Option<u64>,
) {
    let stage = status["stage"].as_str().unwrap_or("not_started");
    let credentials = status["creds_present"].as_bool().unwrap_or(false);
    let action = if matches!(stage, "complete" | "legacy_eoa") {
        None
    } else if credentials && stage == "fund" {
        Some(format!("fund/{wallet}/{account}/new"))
    } else if let Some(after) = login_retry_after_ms.filter(|_| !credentials) {
        status["retry_after_ms"] = serde_json::json!(after);
        status["message"] = serde_json::json!(
            "The last CLOB login failed after the owner approved it, so that approval is spent. Wait until retry_after_ms, then write begin."
        );
        Some(format!("onboard/{wallet}/{account}/begin"))
    } else if approval_pending {
        status["message"] = serde_json::json!(
            "Complete or cancel the existing owner approval, then retry the same action."
        );
        Some(format!(
            "onboard/{wallet}/{account}/{}",
            if credentials { "continue" } else { "begin" }
        ))
    } else if !credentials {
        Some(format!("onboard/{wallet}/{account}/begin"))
    } else {
        Some(format!("onboard/{wallet}/{account}/continue"))
    };
    status["next_action"] = serde_json::json!(action);
}

pub fn stored_status_for_wallet(
    wallet: &str,
    owner: Address,
) -> Result<serde_json::Value, DispatchResponse> {
    match store_get(&format!("onboard/{wallet}/status.json"))
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    {
        Some(status) => {
            let expected = owner.to_checksum(None);
            let Some(stored_owner) = status.get("owner").and_then(serde_json::Value::as_str) else {
                return Err(error(-4, "stored onboarding status is missing owner"));
            };
            let stored_owner = stored_owner
                .parse::<Address>()
                .map_err(|e| error(-4, format!("stored onboarding owner parse: {e}")))?
                .to_checksum(None);
            if stored_owner != expected {
                return Err(error(
                    -3,
                    "stored onboarding status belongs to a different wallet owner",
                ));
            }
            Ok(status)
        }
        None => Ok(local_onboard_status(
            wallet,
            owner,
            "not_started",
            false,
            false,
            "write begin to mint or derive CLOB credentials",
        )),
    }
}

pub fn fundable_deposit_wallet(wallet: &str, owner: Address) -> Result<Address, DispatchResponse> {
    let status = stored_status_for_wallet(wallet, owner)?;
    fundable_deposit_wallet_from_status(&status).ok_or_else(|| {
        error(
            -3,
            "deposit wallet is not factory-resolved; obtain credentials with begin, then write onboard/<wallet>/<index>/continue before funding",
        )
    })
}

pub fn tradeable_deposit_wallet(wallet: &str, owner: Address) -> Result<Address, DispatchResponse> {
    let status = local_status_for_wallet(wallet, owner)?;
    let deposit = fundable_deposit_wallet_from_status(&status).ok_or_else(|| {
        error(
            -3,
            "deposit wallet is not factory-resolved; obtain credentials with begin, then write onboard/<wallet>/<index>/continue before posting",
        )
    })?;
    if status.get("stage").and_then(serde_json::Value::as_str) != Some("complete")
        || !status
            .get("tradeable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    {
        return Err(error(
            -3,
            "wallet onboarding is not complete; read onboard/<wallet>/<index>/status.json and complete deploy, fund, approve, credentials, and CLOB sync before posting",
        ));
    }
    Ok(deposit)
}

pub fn fundable_deposit_wallet_from_status(status: &serde_json::Value) -> Option<Address> {
    let deposit = status
        .get("deposit_wallet")
        .and_then(|value| value.get("address"))
        .and_then(serde_json::Value::as_str)?;
    let source = status
        .get("deposit_wallet")
        .and_then(|value| value.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let fundable = status
        .get("deposit_wallet")
        .and_then(|value| value.get("fundable"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if source != "live_factory_resolved" || !fundable {
        return None;
    }
    deposit.parse::<Address>().ok()
}

#[cfg(test)]
mod next_action_tests {
    use super::*;

    #[test]
    fn guidance_tracks_credentials_funding_approval_and_completion() {
        for (stage, credentials, pending, expected) in [
            ("not_started", false, false, Some("onboard/w/1/begin")),
            ("not_started", true, false, Some("onboard/w/1/continue")),
            ("approve", true, true, Some("onboard/w/1/continue")),
            ("creds", false, true, Some("onboard/w/1/begin")),
            ("fund", true, false, Some("fund/w/1/new")),
            ("fund", true, true, Some("fund/w/1/new")),
            ("fund", false, true, Some("onboard/w/1/begin")),
            ("complete", true, false, None),
            ("legacy_eoa", true, false, None),
        ] {
            let mut status = serde_json::json!({"stage": stage, "creds_present": credentials, "next_action": "stale", "message": "producer-specific status"});
            project_next_action(&mut status, "w", 1, pending, None);
            assert_eq!(
                status["next_action"],
                serde_json::json!(expected),
                "{stage}"
            );
            if pending && !(credentials && stage == "fund") {
                assert!(
                    status["message"]
                        .as_str()
                        .unwrap()
                        .contains("existing owner approval")
                );
            } else {
                assert_eq!(status["message"], "producer-specific status");
            }
        }
    }

    #[test]
    fn failed_login_waits_until_its_approval_expires() {
        let mut status = serde_json::json!({"stage": "not_started", "creds_present": false});
        project_next_action(&mut status, "w", 1, false, Some(42));
        assert_eq!(status["next_action"], "onboard/w/1/begin");
        assert_eq!(status["retry_after_ms"], 42);
        assert!(
            status["message"]
                .as_str()
                .unwrap()
                .starts_with("The last CLOB login failed")
        );

        let mut status = serde_json::json!({"stage": "not_started", "creds_present": true});
        project_next_action(&mut status, "w", 1, false, Some(42));
        assert_eq!(status["next_action"], "onboard/w/1/continue");
        assert!(status.get("retry_after_ms").is_none());
    }
}
