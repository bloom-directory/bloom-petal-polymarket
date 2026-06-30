use crate::*;

use crate::bloom_petal_sdk::{DispatchResponse, HttpRequest, SignRequest};
use crate::eip712::{FACTORY, PUSD, clob_auth_signing_hash};
use crate::polymarket::{
    Credentials, POLYGON, Result, derive_deposit_wallet_address, validate_wallet_name,
};
use crate::signer::{POLY_ADDRESS, POLY_NONCE, POLY_SIGNATURE, POLY_TIMESTAMP};
use crate::wallet::{V2_APPROVAL_LABELS, v2_approval_calls};
use alloy::primitives::{Address, U256};
pub(crate) fn begin_onboarding(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    if let Err(resp) = check_geoblock() {
        return resp;
    }
    let deposit = match predict_deposit_wallet(owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let timestamp = now_secs();
    let hash = clob_auth_signing_hash(owner, timestamp, CLOB_AUTH_NONCE, POLYGON);
    let signature = match bloom_petal_sdk::sign_hash(&SignRequest {
        wallet: wallet.into(),
        hash32: hash.into(),
        purpose: "polymarket.clob_auth".into(),
    }) {
        Ok(sig) if sig.len() == 65 => format!("0x{}", hex::encode(sig)),
        Ok(sig) => return error(-4, format!("sign_hash returned {} bytes", sig.len())),
        Err(e) => return sdk_error(e),
    };
    let headers = [
        (POLY_ADDRESS, format!("{owner:#x}")),
        (POLY_NONCE, CLOB_AUTH_NONCE.to_string()),
        (POLY_SIGNATURE, signature),
        (POLY_TIMESTAMP, timestamp.to_string()),
    ];
    let creds = match clob_auth_request("POST", "/auth/api-key", &headers) {
        Ok(creds) => creds,
        Err(DispatchResponse::Error { code: -4, .. }) => {
            match clob_auth_request("GET", "/auth/derive-api-key", &headers) {
                Ok(creds) => creds,
                Err(resp) => return resp,
            }
        }
        Err(resp) => return resp,
    };
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("creds/{wallet}/clob.json"), &creds, true)
    {
        return error(-4, "failed to store CLOB credentials");
    }
    match run_onboard_stages(wallet, owner, deposit, &creds) {
        Ok(status) => store_put_json(&format!("onboard/{wallet}/status.json"), &status, false),
        Err(resp) => {
            let _ = persist_onboard_failure(wallet, owner, deposit, &resp);
            resp
        }
    }
}

pub(crate) fn check_geoblock() -> Result<(), DispatchResponse> {
    let resp = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url: format!("{POLYMARKET_WEB}/api/geoblock"),
            headers: Vec::new(),
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(|e| {
        error(
            -3,
            format!(
                "could not verify region availability (geoblock check failed: {}); refusing",
                e.message()
            ),
        )
    })?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -3,
            format!(
                "could not verify region availability (geoblock status {}); refusing",
                resp.status
            ),
        ));
    }
    let status: GeoblockStatus = serde_json::from_slice(&resp.body).map_err(|e| {
        error(
            -3,
            format!("could not verify region availability (geoblock JSON: {e}); refusing"),
        )
    })?;
    if status.blocked {
        return Err(error(
            -3,
            format!(
                "Polymarket is unavailable in your region (country={}, region={}); refusing to onboard",
                status.country, status.region
            ),
        ));
    }
    Ok(())
}

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
    wallet: &'a str,
    owner: Address,
    deposit: Address,
    stage: &'a str,
    running: bool,
    creds_present: bool,
    tradeable: bool,
    message: &'a str,
    probes: serde_json::Value,
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

pub(crate) fn run_onboard_stages(
    wallet: &str,
    owner: Address,
    deposit: Address,
    creds: &Credentials,
) -> Result<serde_json::Value, DispatchResponse> {
    let mut deploy_tx_id = stored_status_for_wallet(wallet, owner)
        .ok()
        .and_then(|status| {
            status
                .get("deploy_tx_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    let mut approve_tx_id = stored_status_for_wallet(wallet, owner)
        .ok()
        .and_then(|status| {
            status
                .get("approve_tx_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });

    if !read_chain_deposit_wallet_deployed(deposit)? {
        let _builder = ensure_builder_credentials(wallet, owner, creds)?;
        let tx = relayer_submit_with_builder_repair(
            wallet,
            owner,
            creds,
            serde_json::json!({
                "type": "WALLET-CREATE",
                "from": owner.to_checksum(None),
                "to": FACTORY.to_checksum(None),
            }),
        )?;
        deploy_tx_id = Some(tx.id.clone());
        persist_onboard_status(
            wallet,
            owner,
            deposit,
            true,
            OnboardStatusExtra {
                stage: Some("deploy"),
                deploy_tx_id: deploy_tx_id.clone(),
                approve_tx_id: approve_tx_id.clone(),
                in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
                relayer_auth: Some("builder_key_auto"),
                last_error: None,
            },
        )?;
        let confirmed = match relayer_poll_confirmed(&tx) {
            Ok(done) => done,
            Err(resp) => {
                let msg = dispatch_error_message(&resp);
                persist_onboard_status(
                    wallet,
                    owner,
                    deposit,
                    true,
                    OnboardStatusExtra {
                        stage: Some("deploy"),
                        deploy_tx_id,
                        approve_tx_id,
                        in_flight_deadline_ms: None,
                        relayer_auth: Some("builder_key_auto"),
                        last_error: Some(msg),
                    },
                )?;
                return Err(resp);
            }
        };
        deploy_tx_id = Some(confirmed.id);
        if !read_chain_deposit_wallet_deployed(deposit)? {
            let msg = "relayer confirmed the deploy but no proxy implementation exists at the deposit wallet".to_string();
            persist_onboard_status(
                wallet,
                owner,
                deposit,
                true,
                OnboardStatusExtra {
                    stage: Some("deploy"),
                    deploy_tx_id,
                    approve_tx_id,
                    in_flight_deadline_ms: None,
                    relayer_auth: Some("builder_key_auto"),
                    last_error: Some(msg.clone()),
                },
            )?;
            return Err(error(-4, msg));
        }
    }

    let pusd_balance = read_chain_erc20_balance(PUSD, deposit)?;
    if pusd_balance.is_zero() {
        return persist_onboard_status(
            wallet,
            owner,
            deposit,
            true,
            OnboardStatusExtra {
                stage: Some("fund"),
                deploy_tx_id,
                approve_tx_id,
                in_flight_deadline_ms: None,
                relayer_auth: Some("builder_key_auto"),
                last_error: None,
            },
        );
    }

    if !read_chain_v2_approvals(deposit)? {
        let _builder = ensure_builder_credentials(wallet, owner, creds)?;
        let nonce = relayer_wallet_nonce(owner)?;
        let deadline = now_secs().saturating_add(BATCH_DEADLINE_SECS);
        let tx = relayer_submit_with_builder_repair(
            wallet,
            owner,
            creds,
            relayer_batch_body(wallet, owner, deposit, nonce, deadline)?,
        )?;
        approve_tx_id = Some(tx.id.clone());
        persist_onboard_status(
            wallet,
            owner,
            deposit,
            true,
            OnboardStatusExtra {
                stage: Some("approve"),
                deploy_tx_id: deploy_tx_id.clone(),
                approve_tx_id: approve_tx_id.clone(),
                in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
                relayer_auth: Some("builder_key_auto"),
                last_error: None,
            },
        )?;
        let confirmed = match relayer_poll_confirmed(&tx) {
            Ok(done) => done,
            Err(resp) => {
                let msg = dispatch_error_message(&resp);
                persist_onboard_status(
                    wallet,
                    owner,
                    deposit,
                    true,
                    OnboardStatusExtra {
                        stage: Some("approve"),
                        deploy_tx_id,
                        approve_tx_id,
                        in_flight_deadline_ms: None,
                        relayer_auth: Some("builder_key_auto"),
                        last_error: Some(msg),
                    },
                )?;
                return Err(resp);
            }
        };
        approve_tx_id = Some(confirmed.id);
        if !read_chain_v2_approvals(deposit)? {
            let msg = "approvals confirmed but on-chain allowances are still missing".to_string();
            persist_onboard_status(
                wallet,
                owner,
                deposit,
                true,
                OnboardStatusExtra {
                    stage: Some("approve"),
                    deploy_tx_id,
                    approve_tx_id,
                    in_flight_deadline_ms: None,
                    relayer_auth: Some("builder_key_auto"),
                    last_error: Some(msg.clone()),
                },
            )?;
            return Err(error(-4, msg));
        }
    }

    persist_onboard_status(
        wallet,
        owner,
        deposit,
        true,
        OnboardStatusExtra {
            stage: Some("sync"),
            deploy_tx_id: deploy_tx_id.clone(),
            approve_tx_id: approve_tx_id.clone(),
            in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
            relayer_auth: Some("builder_key_auto"),
            last_error: None,
        },
    )?;
    clob_l2_get_json(
        owner,
        creds,
        "/balance-allowance/update",
        &[("asset_type", "COLLATERAL"), ("signature_type", "3")],
    )?;
    persist_onboard_status(
        wallet,
        owner,
        deposit,
        true,
        OnboardStatusExtra {
            stage: None,
            deploy_tx_id,
            approve_tx_id,
            in_flight_deadline_ms: None,
            relayer_auth: Some("builder_key_auto"),
            last_error: None,
        },
    )
}

#[derive(Default)]
pub(crate) struct OnboardStatusExtra {
    stage: Option<&'static str>,
    deploy_tx_id: Option<String>,
    approve_tx_id: Option<String>,
    in_flight_deadline_ms: Option<u128>,
    relayer_auth: Option<&'static str>,
    last_error: Option<String>,
}

pub(crate) fn persist_onboard_status(
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

pub(crate) fn persist_onboard_failure(
    wallet: &str,
    owner: Address,
    deposit: Address,
    resp: &DispatchResponse,
) -> Result<serde_json::Value, DispatchResponse> {
    persist_onboard_status(
        wallet,
        owner,
        deposit,
        store_get(&format!("creds/{wallet}/clob.json")).is_some(),
        OnboardStatusExtra {
            last_error: Some(dispatch_error_message(resp)),
            ..OnboardStatusExtra::default()
        },
    )
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

pub(crate) fn approval_preview(wallet: &str, owner: Address) -> serde_json::Value {
    let status = stored_status_for_wallet(wallet, owner).ok();
    let deposit = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("address"))
                .and_then(serde_json::Value::as_str)
        })
        .and_then(|address| address.parse::<Address>().ok())
        .unwrap_or_else(|| derive_deposit_wallet_address(&owner, POLYGON));
    let deposit_source = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("source"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("local_estimate_unverified");
    let raw_deposit_fundable = status
        .as_ref()
        .and_then(|status| {
            status
                .get("deposit_wallet")
                .and_then(|value| value.get("fundable"))
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false);
    let deposit_fundable = raw_deposit_fundable && deposit_source == "live_factory_resolved";
    let calls: Vec<serde_json::Value> = v2_approval_calls()
        .iter()
        .zip(V2_APPROVAL_LABELS)
        .map(|(call, label)| {
            serde_json::json!({
                "label": label,
                "target": format!("{:#x}", call.target),
                "value": call.value.to_string(),
                "data": format!("0x{}", hex::encode(call.data.as_ref())),
            })
        })
        .collect();
    serde_json::json!({
        "wallet": wallet,
        "owner": format!("{owner:#x}"),
        "deposit_wallet": deposit.to_checksum(None),
        "deposit_wallet_source": deposit_source,
        "deposit_wallet_fundable": deposit_fundable,
        "warning": if deposit_fundable {
            serde_json::Value::Null
        } else {
            serde_json::Value::String("do not fund this locally derived estimate; full onboarding must resolve the live factory address before funding or approvals".into())
        },
        "chain_id": POLYGON,
        "calls": calls,
        "signing": "preview_only"
    })
}
