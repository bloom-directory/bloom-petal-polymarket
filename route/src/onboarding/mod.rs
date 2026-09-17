use crate::prelude::*;

use crate::polymarket::signer::{POLY_ADDRESS, POLY_NONCE, POLY_SIGNATURE, POLY_TIMESTAMP};
use crate::polymarket::signing::clob_auth_action_and_hash;
use crate::polymarket::{Credentials, validate_wallet_name};
use petal::sdk::DispatchResponse;

mod flow;
mod persistence;
mod status;

pub use flow::run_onboard_stages;

/// Store key of the owner-visible approval artifact shared by the onboarding
/// signatures; only one of them is ever pending at a time.
pub fn approval_key(wallet: &str) -> String {
    format!("onboard/{wallet}/approval.json")
}
pub use persistence::{OnboardStatusExtra, persist_onboard_failure, persist_onboard_status};
pub use status::{
    LiveOnboardStatus, fundable_deposit_wallet, fundable_deposit_wallet_from_status,
    local_onboard_status, local_onboard_status_with_live_deposit, local_status_for_wallet,
    preserve_onboard_metadata, refreshed_live_onboard_status, stored_status_for_wallet,
    tradeable_deposit_wallet,
};

pub fn begin_onboarding(ctx: &petal::Ctx, wallet: &str) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let (_, chain_id) = match crate::runtime_config::chain() {
        Ok(chain) => chain,
        Err(err) => return error(-4, err),
    };
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let relayer_config = match crate::relayer_config::load_relayer_config() {
        Ok(config) => config,
        Err(resp) => return resp,
    };
    if !relayer_config.legacy_eoa_mode
        && let crate::relayer_config::RelayerAuth::Disabled { reason } = relayer_config.auth()
    {
        return error(-3, reason);
    }
    let deposit = if relayer_config.legacy_eoa_mode {
        None
    } else {
        match predict_deposit_wallet(owner) {
            Ok(deposit) => Some(deposit),
            Err(resp) => return resp,
        }
    };
    let approval_key = approval_key(wallet);
    let review_key = format!("onboard/{wallet}/review_intent.json");
    let existing_creds: Option<Credentials> =
        match petal::sdk::store_get(&format!("creds/{wallet}/clob.json"), MAX_STORE_BYTES) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(creds) => Some(creds),
                Err(err) => return error(-4, format!("corrupt credentials: {err}")),
            },
            Err(petal::sdk::SdkError::Host(petal::sdk::HostStatus::NotFound)) => None,
            Err(err) => return sdk_error(err),
        };
    if let Some(creds) = existing_creds {
        // CLOB credentials are durable. Once they exist, onboarding resumes at
        // the live wallet stages instead of preparing another L1 authentication
        // signature. Retire a stale CLOB-auth approval or an unreadable one; a
        // pending deposit-wallet batch approval belongs to the funded stage.
        let stale_clob_approval = match pending_approval_selector(&approval_key) {
            Ok(selector) => matches!(selector.as_deref(), Some(kind) if kind != "exact"),
            Err(resp) => return resp,
        };
        if stale_clob_approval {
            for key in [&approval_key, &review_key] {
                match petal::sdk::store_del(key) {
                    Ok(()) | Err(petal::sdk::SdkError::Host(petal::sdk::HostStatus::NotFound)) => {}
                    Err(err) => return sdk_error(err),
                }
            }
        }
        if relayer_config.legacy_eoa_mode {
            return store_put_json(
                &format!("onboard/{wallet}/status.json"),
                &crate::relayer_config::legacy_eoa_status(wallet, owner),
                false,
            );
        }
        let deposit = deposit.expect("non-legacy deposit");
        return match run_onboard_stages(ctx, wallet, owner, deposit, &creds) {
            Ok(status) => store_put_json(&format!("onboard/{wallet}/status.json"), &status, false),
            Err(resp) => {
                let _ = persist_onboard_failure(wallet, owner, deposit, &resp);
                resp
            }
        };
    }
    // CLOB L1 auth carries Polymarket's server timestamp, which goes stale
    // faster than an owner ceremony completes. Build it fresh on every attempt
    // and sign it under a single-use approval that is not bound to the bytes.
    // The deposit-wallet approval batch is prepared and approved later, in the
    // funded stage, so its one-hour deadline starts when it can be submitted.
    let timestamp = match clob_server_time() {
        Ok(timestamp) => timestamp,
        Err(resp) => return resp,
    };
    let action = clob_auth_action_and_hash(owner, timestamp, CLOB_AUTH_NONCE, chain_id);
    let review = serde_json::json!({
        "operation": "clob_auth",
        "owner": owner.to_checksum(None),
        "chain_id": chain_id,
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
        action.signing_preimage,
        action.signing_hash,
        serde_json::json!({
            "timestamp": action.timestamp,
            "nonce": action.nonce,
            "chain_id": chain_id,
            "review_intent_hash": review_hash,
        }),
    );
    let signature = match sign_prepared_reusable(ctx, wallet, &prepared, &approval_key) {
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
    let _ = petal::sdk::store_del(&approval_key);
    if relayer_config.legacy_eoa_mode {
        return store_put_json(
            &format!("onboard/{wallet}/status.json"),
            &crate::relayer_config::legacy_eoa_status(wallet, owner),
            false,
        );
    }
    let deposit = deposit.expect("non-legacy deposit");
    match run_onboard_stages(ctx, wallet, owner, deposit, &creds) {
        Ok(status) => store_put_json(&format!("onboard/{wallet}/status.json"), &status, false),
        Err(resp) => {
            let _ = persist_onboard_failure(wallet, owner, deposit, &resp);
            resp
        }
    }
}
