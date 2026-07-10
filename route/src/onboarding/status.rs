use crate::prelude::*;

use crate::polymarket::eip712::PUSD;
use crate::polymarket::{POLYGON, Result, derive_deposit_wallet_address};
use alloy::primitives::{Address, U256};
use petal::sdk::DispatchResponse;

pub(crate) fn local_onboard_status(
    wallet: &str,
    owner: Address,
    stage: &str,
    running: bool,
    creds_present: bool,
    message: &str,
) -> serde_json::Value {
    let deposit = derive_deposit_wallet_address(&owner, POLYGON);
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
            "required": true,
            "preview_path": format!("onboard/{wallet}/approvals.json")
        },
        "message": message
    })
}

pub(crate) struct LiveOnboardStatus<'a> {
    pub(crate) wallet: &'a str,
    pub(crate) owner: Address,
    pub(crate) deposit: Address,
    pub(crate) stage: &'a str,
    pub(crate) running: bool,
    pub(crate) creds_present: bool,
    pub(crate) tradeable: bool,
    pub(crate) message: &'a str,
    pub(crate) probes: serde_json::Value,
}

pub(crate) fn local_onboard_status_with_live_deposit(
    status: LiveOnboardStatus<'_>,
) -> serde_json::Value {
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
            "required": true,
            "preview_path": format!("onboard/{}/approvals.json", status.wallet)
        },
        "probes": status.probes,
        "message": status.message
    })
}

pub(crate) fn refreshed_live_onboard_status(
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
        read_chain_v2_approvals(deposit)?
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
            "deposit wallet holds pUSD; waiting for V2 exchange and adapter approvals",
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

pub(crate) fn local_status_for_wallet(
    wallet: &str,
    owner: Address,
) -> Result<serde_json::Value, DispatchResponse> {
    let status = stored_status_for_wallet(wallet, owner)?;
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
    let creds_present = store_get(&format!("creds/{wallet}/clob.json")).is_some();
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

pub(crate) fn preserve_onboard_metadata(
    previous: &serde_json::Value,
    refreshed: &mut serde_json::Value,
) {
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

pub(crate) fn stored_status_for_wallet(
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

pub(crate) fn fundable_deposit_wallet(
    wallet: &str,
    owner: Address,
) -> Result<Address, DispatchResponse> {
    let status = stored_status_for_wallet(wallet, owner)?;
    fundable_deposit_wallet_from_status(&status).ok_or_else(|| {
        error(
            -3,
            "deposit wallet is not factory-resolved; write onboard/<wallet>/begin before funding",
        )
    })
}

pub(crate) fn tradeable_deposit_wallet(
    wallet: &str,
    owner: Address,
) -> Result<Address, DispatchResponse> {
    let status = local_status_for_wallet(wallet, owner)?;
    let deposit = fundable_deposit_wallet_from_status(&status).ok_or_else(|| {
        error(
            -3,
            "deposit wallet is not factory-resolved; write onboard/<wallet>/begin before posting",
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
            "wallet onboarding is not complete; read onboard/<wallet>/status.json and complete deploy, fund, approve, credentials, and CLOB sync before posting",
        ));
    }
    Ok(deposit)
}

pub(crate) fn fundable_deposit_wallet_from_status(status: &serde_json::Value) -> Option<Address> {
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
