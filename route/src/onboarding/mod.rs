use crate::prelude::*;

use crate::polymarket::signer::{POLY_ADDRESS, POLY_NONCE, POLY_SIGNATURE, POLY_TIMESTAMP};
use crate::polymarket::signing::clob_auth_action_and_hash;
use crate::polymarket::{POLYGON, validate_wallet_name};
use petal::sdk::DispatchResponse;

mod flow;
mod persistence;
mod status;

pub(crate) use flow::run_onboard_stages;
pub(crate) use persistence::{OnboardStatusExtra, persist_onboard_failure, persist_onboard_status};
pub(crate) use status::{
    LiveOnboardStatus, fundable_deposit_wallet, fundable_deposit_wallet_from_status,
    local_onboard_status, local_onboard_status_with_live_deposit, local_status_for_wallet,
    preserve_onboard_metadata, refreshed_live_onboard_status, stored_status_for_wallet,
    tradeable_deposit_wallet,
};

pub(crate) fn begin_onboarding(wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let deposit = match predict_deposit_wallet(owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let prepared_key = format!("onboard/{wallet}/prepared_clob_auth.json");
    let approval_key = format!("onboard/{wallet}/approval.json");
    let review_key = format!("onboard/{wallet}/review_intent.json");
    let prepared = match load_prepared_signing(&prepared_key) {
        Ok(Some(prepared)) => prepared,
        Ok(None) => {
            let timestamp = match clob_server_time() {
                Ok(timestamp) => timestamp,
                Err(resp) => return resp,
            };
            let action = clob_auth_action_and_hash(owner, timestamp, CLOB_AUTH_NONCE, POLYGON);
            let review = serde_json::json!({
                "operation": "clob_auth",
                "owner": owner.to_checksum(None),
                "chain_id": POLYGON,
                "timestamp": action.timestamp,
                "nonce": action.nonce,
                "message": action.message,
                "signing_hash": format!("{:#x}", action.signing_hash),
            });
            let review_hash = match store_review_intent(&review_key, &review) {
                Ok(hash) => hash,
                Err(resp) => return resp,
            };
            let prepared = PreparedSigning::new(
                "clob_auth",
                "polymarket.onboard",
                owner,
                action.signing_hash,
                serde_json::json!({
                    "timestamp": action.timestamp,
                    "nonce": action.nonce,
                    "chain_id": POLYGON,
                    "review_intent_hash": review_hash,
                }),
            );
            if let Err(resp) = store_prepared_signing(&prepared_key, &prepared) {
                return resp;
            }
            prepared
        }
        Err(resp) => return resp,
    };
    if prepared.operation != "clob_auth" || prepared.owner != owner.to_checksum(None) {
        return error(-4, "prepared CLOB auth does not match wallet");
    }
    let timestamp = match prepared
        .preimage
        .get("timestamp")
        .and_then(serde_json::Value::as_u64)
    {
        Some(timestamp) => timestamp,
        None => return error(-4, "prepared CLOB auth is missing timestamp"),
    };
    let review_hash = match prepared
        .preimage
        .get("review_intent_hash")
        .and_then(serde_json::Value::as_str)
    {
        Some(hash) => hash,
        None => return error(-4, "prepared CLOB auth is missing review intent hash"),
    };
    if let Err(resp) = verify_review_intent(&review_key, review_hash) {
        return resp;
    }
    let signature = match sign_prepared(wallet, &prepared, &approval_key) {
        Ok(signature) => format!("0x{}", hex::encode(signature)),
        Err(resp) => return resp,
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
    let _ = petal::sdk::store_del(&prepared_key);
    let _ = petal::sdk::store_del(&approval_key);
    match run_onboard_stages(wallet, owner, deposit, &creds) {
        Ok(status) => store_put_json(&format!("onboard/{wallet}/status.json"), &status, false),
        Err(resp) => {
            let _ = persist_onboard_failure(wallet, owner, deposit, &resp);
            resp
        }
    }
}
