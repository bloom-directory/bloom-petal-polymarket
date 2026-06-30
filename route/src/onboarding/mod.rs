use crate::prelude::*;

use petal::sdk::{DispatchResponse, SignRequest};
use crate::polymarket::eip712::clob_auth_signing_hash;
use crate::polymarket::{POLYGON, validate_wallet_name};
use crate::polymarket::signer::{POLY_ADDRESS, POLY_NONCE, POLY_SIGNATURE, POLY_TIMESTAMP};

mod flow;
mod geoblock;
mod persistence;
mod status;

pub(crate) use flow::run_onboard_stages;
pub(crate) use geoblock::check_geoblock;
pub(crate) use persistence::{
    OnboardStatusExtra, persist_onboard_failure, persist_onboard_status,
};
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
    if let Err(resp) = check_geoblock() {
        return resp;
    }
    let deposit = match predict_deposit_wallet(owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let timestamp = now_secs();
    let hash = clob_auth_signing_hash(owner, timestamp, CLOB_AUTH_NONCE, POLYGON);
    let signature = match petal::sdk::sign_hash(&SignRequest {
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
