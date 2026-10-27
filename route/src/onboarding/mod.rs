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
/// Store key holding when a CLOB login whose post-signing step failed may be
/// retried. Machine keeps the spent single-use grant for APPROVAL_TTL_MS.
fn login_retry_key(wallet: &str) -> String {
    format!("onboard/{wallet}/login_retry_after.json")
}
const LOGIN_RETRY_WAIT_MS: u64 = 5 * 60 * 1000;

/// When a CLOB login may be retried, if a failed attempt is still waiting.
pub fn login_retry_after_ms(wallet: &str) -> Option<u64> {
    store_get(&login_retry_key(wallet))
        .and_then(|bytes| serde_json::from_slice::<u64>(&bytes).ok())
        .filter(|after| *after > now_millis() as u64)
}

fn login_failed_after_signing(response: DispatchResponse, retry_after_ms: u64) -> DispatchResponse {
    let detail = match response {
        DispatchResponse::Error { message, .. } => message,
        other => format!("{other:?}"),
    };
    error(
        -4,
        format!(
            "CLOB login failed after the owner approved it ({detail}); that single-use approval is spent, so wait until {retry_after_ms} ms (status.json retry_after_ms), then write begin again"
        ),
    )
}

pub use persistence::{OnboardStatusExtra, persist_onboard_failure, persist_onboard_status};
pub use status::{
    LiveOnboardStatus, fundable_deposit_wallet, fundable_deposit_wallet_from_status,
    local_onboard_status, local_onboard_status_with_live_deposit, local_status_for_wallet,
    preserve_onboard_metadata, project_next_action, refreshed_live_onboard_status,
    stored_status_for_wallet, tradeable_deposit_wallet,
};

pub fn begin_onboarding(
    ctx: &petal::Ctx,
    wallet: &str,
    account: &petal::WalletAccount,
) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let (_, chain_id) = match crate::runtime_config::chain() {
        Ok(chain) => chain,
        Err(err) => return error(-4, err),
    };
    let owner = match wallet_address(account) {
        Ok(address) => address,
        Err(resp) => return resp,
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
    if existing_creds.is_some() {
        return DispatchResponse::Write;
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
        "polymarket.clob_auth.v1",
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
    // CLOB L1 auth proves control of the owner address to mint API
    // credentials. It moves no value and authorizes no spender.
    let signature = match sign_prepared_reusable(
        ctx,
        wallet,
        &prepared,
        &petal::ClaimEffects::moves_nothing(),
        &approval_key,
    ) {
        // v is rendered as 27/28, the form Polymarket's own clients send.
        Ok(signature) => signature.to_string(),
        Err(resp) => return resp,
    };
    let headers = [
        (POLY_ADDRESS, format!("{owner:#x}")),
        (POLY_NONCE, CLOB_AUTH_NONCE.to_string()),
        (POLY_SIGNATURE, signature),
        (POLY_TIMESTAMP, timestamp.to_string()),
    ];
    let stored = match clob_auth_request("POST", "/auth/api-key", &headers) {
        Err(DispatchResponse::Error { code: -4, .. }) => {
            clob_auth_request("GET", "/auth/derive-api-key", &headers)
        }
        result => result,
    }
    .and_then(|creds| {
        match store_put_json(&format!("creds/{wallet}/clob.json"), &creds, true) {
            DispatchResponse::Error { .. } => Err(error(-4, "failed to store CLOB credentials")),
            _ => Ok(()),
        }
    });
    if let Err(response) = stored {
        let retry_after_ms = now_millis() as u64 + LOGIN_RETRY_WAIT_MS;
        let _ = store_put_json(&login_retry_key(wallet), &retry_after_ms, false);
        return login_failed_after_signing(response, retry_after_ms);
    }
    let _ = petal::sdk::store_del(&approval_key);
    DispatchResponse::Write
}

pub fn continue_onboarding(
    ctx: &petal::Ctx,
    wallet: &str,
    account: &petal::WalletAccount,
) -> DispatchResponse {
    if let Err(error) = validate_wallet_name(wallet) {
        return crate::prelude::error(-3, error.to_string());
    }
    let owner = match wallet_address(account) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let creds: Credentials =
        match petal::sdk::store_get(&format!("creds/{wallet}/clob.json"), MAX_STORE_BYTES) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(creds) => creds,
                Err(_) => return error(-4, "stored CLOB credentials are corrupt"),
            },
            Err(petal::sdk::SdkError::Host(petal::sdk::HostStatus::NotFound)) => {
                return error(-3, "write begin to obtain CLOB credentials first");
            }
            Err(error) => return sdk_error(error),
        };
    // Retire only a stale CLOB approval. An exact deposit-wallet approval must
    // survive retries of this continuation.
    let key = approval_key(wallet);
    let stale = match pending_approval_selector(&key) {
        Ok(selector) => matches!(selector.as_deref(), Some(kind) if kind != "exact"),
        Err(response) => return response,
    };
    if stale {
        for key in [key, format!("onboard/{wallet}/review_intent.json")] {
            match petal::sdk::store_del(&key) {
                Ok(()) | Err(petal::sdk::SdkError::Host(petal::sdk::HostStatus::NotFound)) => {}
                Err(error) => return sdk_error(error),
            }
        }
    }
    let config = match crate::relayer_config::load_relayer_config() {
        Ok(config) => config,
        Err(response) => return response,
    };
    if config.legacy_eoa_mode {
        return store_put_json(
            &format!("onboard/{wallet}/status.json"),
            &crate::relayer_config::legacy_eoa_status(wallet, owner),
            false,
        );
    }
    if let crate::relayer_config::RelayerAuth::Disabled { reason } = config.auth() {
        return error(-3, reason);
    }
    let deposit = match predict_deposit_wallet(owner) {
        Ok(deposit) => deposit,
        Err(response) => return response,
    };
    match run_onboard_stages(ctx, wallet, owner, deposit, &creds) {
        Ok(status) => store_put_json(&format!("onboard/{wallet}/status.json"), &status, false),
        Err(response) => {
            let _ = persist_onboard_failure(wallet, owner, deposit, &response);
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_failure_after_signing_explains_the_wait() {
        let DispatchResponse::Error { code, message } =
            login_failed_after_signing(error(-2, "clob refused"), 42)
        else {
            panic!("must be an error");
        };
        assert_eq!(code, -4, "Bloom keeps the message of -4, not -2");
        assert!(message.contains("clob refused"));
        assert!(message.contains("wait until 42 ms"));
    }
}
