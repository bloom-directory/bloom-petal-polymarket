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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct LoginAttempt {
    signed: bool,
    completed: bool,
    timestamp: u64,
}

fn login_signing_attempt(attempt: &LoginAttempt) -> String {
    attempt.timestamp.to_string()
}

fn login_attempt_key(wallet: &str) -> String {
    format!("onboard/{wallet}/login_attempt.json")
}

fn parse_login_attempt(bytes: &[u8]) -> Result<LoginAttempt, DispatchResponse> {
    let attempt: LoginAttempt = serde_json::from_slice(bytes)
        .map_err(|e| error(-4, format!("corrupt login attempt: {e}")))?;
    if attempt.timestamp == 0 {
        return Err(error(-4, "invalid login attempt timestamp"));
    }
    Ok(attempt)
}

fn load_login_attempt(wallet: &str) -> Result<Option<LoginAttempt>, DispatchResponse> {
    match petal::sdk::store_get(&login_attempt_key(wallet), MAX_STORE_BYTES) {
        Ok(bytes) => parse_login_attempt(&bytes).map(Some),
        Err(petal::sdk::SdkError::Host(petal::sdk::HostStatus::NotFound)) => Ok(None),
        Err(e) => Err(sdk_error(e)),
    }
}

/// Pending exact approvals always retain their payload. Only an explicit
/// refresh of a signed attempt (or a completed login) starts a new timestamp.
fn select_login_attempt(existing: Option<LoginAttempt>, refresh: bool) -> LoginAttempt {
    existing
        .filter(|attempt| !attempt.completed && !(refresh && attempt.signed))
        .unwrap_or(LoginAttempt {
            signed: false,
            completed: false,
            timestamp: 0,
        })
}

fn current_login_attempt(wallet: &str, refresh: bool) -> Result<LoginAttempt, DispatchResponse> {
    Ok(select_login_attempt(load_login_attempt(wallet)?, refresh))
}

fn complete_login_attempt(wallet: &str, mut attempt: LoginAttempt) -> Result<(), DispatchResponse> {
    attempt.completed = true;
    match store_put_json(&login_attempt_key(wallet), &attempt, false) {
        DispatchResponse::Write => Ok(()),
        response => Err(response),
    }
}

pub fn approval_operations(wallet: &str) -> Result<Vec<String>, DispatchResponse> {
    let key = approval_key(wallet);
    let mut operations = Vec::new();
    if let Some(attempt) = load_login_attempt(wallet)?
        && !attempt.completed
    {
        operations.push(crate::approval::operation_identity(
            &key,
            "polymarket.clob_auth.v1",
            Some(&login_signing_attempt(&attempt)),
        ));
    }
    if let Some(prepared) =
        load_prepared_signing(&format!("onboard/{wallet}/prepared_relayer_batch.json"))?
    {
        operations.push(crate::approval::operation_identity(
            &key,
            "polymarket.onboard",
            Some(&crate::approval::relayer_attempt(&prepared)?),
        ));
    }
    Ok(operations)
}

fn login_failed_after_signing(response: DispatchResponse) -> DispatchResponse {
    let detail = match response {
        DispatchResponse::Error { message, .. } => message,
        other => format!("{other:?}"),
    };
    error(
        -4,
        format!(
            "CLOB login failed after signing ({detail}); write begin with {{\"refresh\":true}} to refresh the login payload"
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
    refresh: bool,
) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let _lock = match acquire_trade_lock(wallet, "onboarding-login") {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
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
    if existing_creds.is_some() && !refresh {
        match load_login_attempt(wallet) {
            Ok(Some(attempt)) if !attempt.completed => {
                if let Err(resp) = complete_login_attempt(wallet, attempt) {
                    return resp;
                }
            }
            Ok(_) => (),
            Err(resp) => return resp,
        }
        return DispatchResponse::Write;
    }
    let mut attempt = match current_login_attempt(wallet, refresh) {
        Ok(attempt) => attempt,
        Err(resp) => return resp,
    };
    // A refreshed server timestamp is a new exact payload, and therefore a new
    // durable operation. Retries of that timestamp retain the same identity.
    let timestamp = if attempt.timestamp != 0 {
        attempt.timestamp
    } else {
        match clob_server_time() {
            Ok(timestamp) => timestamp,
            Err(resp) => return resp,
        }
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
    attempt.timestamp = timestamp;
    if let DispatchResponse::Error { code, message } =
        store_put_json(&login_attempt_key(wallet), &attempt, false)
    {
        return error(code, message);
    }
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
        crate::polymarket::eip712::clob_auth_typed_data(
            owner,
            timestamp,
            CLOB_AUTH_NONCE,
            chain_id,
        ),
    );
    let prepared_action = match crate::approval::action_for_prepared(&prepared) {
        Ok(action) => action.with_spend(petal::signing::Spend::None),
        Err(resp) => return resp,
    };
    let signature = match crate::approval::sign_action_for(
        ctx,
        wallet,
        &prepared_action,
        &approval_key,
        Some(&login_signing_attempt(&attempt)),
    ) {
        Ok(signature) => {
            match crate::approval::verified_signature(owner, action.signing_hash, &signature) {
                Ok(signature) => signature.to_string(),
                Err(resp) => return resp,
            }
        }
        Err(resp) => return resp,
    };
    attempt.signed = true;
    if let DispatchResponse::Error { code, message } =
        store_put_json(&login_attempt_key(wallet), &attempt, false)
    {
        return error(code, message);
    }
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
        return login_failed_after_signing(response);
    }
    match complete_login_attempt(wallet, attempt) {
        Ok(()) => DispatchResponse::Write,
        Err(resp) => resp,
    }
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
    fn exact_login_approval_retries_keep_the_frozen_payload_until_signed() {
        let pending = LoginAttempt {
            signed: false,
            completed: false,
            timestamp: 100,
        };
        let reloaded = parse_login_attempt(&serde_json::to_vec(&pending).unwrap()).unwrap();
        assert_eq!(select_login_attempt(Some(reloaded.clone()), false), pending);
        // Retrying the same explicit-refresh write must also resume the approval.
        assert_eq!(select_login_attempt(Some(reloaded), true), pending);
        let signed = LoginAttempt {
            signed: true,
            ..pending.clone()
        };
        assert_eq!(select_login_attempt(Some(signed.clone()), false), signed);
        assert_eq!(select_login_attempt(Some(signed), true).timestamp, 0);
        let complete = LoginAttempt {
            signed: true,
            completed: true,
            ..pending
        };
        assert_eq!(select_login_attempt(Some(complete), true).timestamp, 0);
    }

    #[test]
    fn login_attempt_identity_binds_refresh_and_survives_reload() {
        let first = LoginAttempt {
            signed: false,
            completed: false,
            timestamp: 100,
        };
        let reload = parse_login_attempt(&serde_json::to_vec(&first).unwrap()).unwrap();
        let key = approval_key("test");
        let at_first = clob_auth_action_and_hash(
            alloy::primitives::Address::repeat_byte(1),
            100,
            CLOB_AUTH_NONCE,
            crate::polymarket::POLYGON,
        );
        let refreshed = clob_auth_action_and_hash(
            alloy::primitives::Address::repeat_byte(1),
            200,
            CLOB_AUTH_NONCE,
            crate::polymarket::POLYGON,
        );
        assert_ne!(at_first.signing_hash, refreshed.signing_hash);
        assert_eq!(
            crate::approval::operation_identity(
                &key,
                "polymarket.clob_auth.v1",
                Some(&login_signing_attempt(&first))
            ),
            crate::approval::operation_identity(
                &key,
                "polymarket.clob_auth.v1",
                Some(&login_signing_attempt(&reload))
            )
        );
        let mut fresh = reload.clone();
        fresh.timestamp += 1;
        assert_ne!(
            login_signing_attempt(&reload),
            login_signing_attempt(&fresh)
        );
        let same_payload = LoginAttempt {
            signed: true,
            completed: true,
            timestamp: first.timestamp,
        };
        assert_eq!(
            login_signing_attempt(&first),
            login_signing_attempt(&same_payload)
        );
    }

    #[test]
    fn login_attempt_rejects_invalid_timestamp() {
        assert!(
            parse_login_attempt(br#"{"timestamp":0,"completed":false,"signed":false}"#).is_err()
        );
        assert!(
            parse_login_attempt(br#"{"timestamp":"bad","completed":false,"signed":false}"#)
                .is_err()
        );
    }

    #[test]
    fn login_failure_after_signing_allows_immediate_refresh() {
        let DispatchResponse::Error { code, message } =
            login_failed_after_signing(error(-2, "clob refused"))
        else {
            panic!("must be an error");
        };
        assert_eq!(code, -4, "Bloom keeps the message of -4, not -2");
        assert!(message.contains("clob refused"));
        assert!(message.contains("refresh"));
    }
}
