use crate::prelude::*;

use crate::polymarket::eip712::{FACTORY, PUSD};
use crate::polymarket::{Credentials, Result};
use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HostStatus};

pub fn run_onboard_stages(
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
        let tx = if let Some(id) = deploy_tx_id.as_deref() {
            relayer_transaction(id)?
        } else {
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
            tx
        };
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
                        in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
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
        if let Some(id) = approve_tx_id.as_deref() {
            let previous = relayer_transaction(id)?;
            if previous.is_confirmed() {
                // The historical batch completed but live authority is absent
                // (for example after an explicit revoke). Retire it so a new
                // nonce-bound approval batch can be prepared.
                approve_tx_id = None;
                for key in [
                    format!("onboard/{wallet}/prepared_relayer_batch.json"),
                    format!("onboard/{wallet}/approval.json"),
                ] {
                    match petal::sdk::store_del(&key) {
                        Ok(()) | Err(petal::sdk::SdkError::Host(HostStatus::NotFound)) => {}
                        Err(err) => return Err(sdk_error(err)),
                    }
                }
            }
        }
        let tx = if let Some(id) = approve_tx_id.as_deref() {
            relayer_transaction(id)?
        } else {
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
            tx
        };
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
                        in_flight_deadline_ms: Some(onboard_in_flight_deadline_ms()),
                        relayer_auth: Some("builder_key_auto"),
                        last_error: Some(msg),
                    },
                )?;
                return Err(resp);
            }
        };
        approve_tx_id = Some(confirmed.id);
        let _ = petal::sdk::store_del(&format!("onboard/{wallet}/prepared_relayer_batch.json"));
        let _ = petal::sdk::store_del(&format!("onboard/{wallet}/approval.json"));
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
