use crate::prelude::*;

use crate::polymarket::signer::{POLY_ADDRESS, POLY_NONCE, POLY_SIGNATURE, POLY_TIMESTAMP};
use crate::polymarket::signing::clob_auth_action_and_hash;
use crate::polymarket::{Credentials, validate_wallet_name};
use petal::sdk::DispatchResponse;
use petal::signing::{Attempt, AttemptState, CtxHost, eip712_action, sign_or_resume};

mod flow;
mod persistence;
mod status;

pub use flow::run_onboard_stages;

fn login_attempt(
    host: &mut impl petal::signing::SigningHost,
    wallet: &str,
    refresh: bool,
    build: impl Fn() -> Result<(petal::signing::PreparedAction, u64), petal::signing::SigningError>,
) -> Result<Attempt<u64>, petal::signing::SignHalt> {
    let attempt = Attempt::open(host, &format!("onboard/{wallet}/login"), &build)?;
    match attempt.state(host, wallet)? {
        AttemptState::Expired | AttemptState::Cancelled => attempt.renew(host, wallet, |_| build()),
        AttemptState::Signed | AttemptState::PackageChanged if refresh => {
            attempt.renew(host, wallet, |_| build())
        }
        _ => Ok(attempt),
    }
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
        return DispatchResponse::Write;
    }
    let mut host = CtxHost(ctx);
    let build = || -> Result<_, petal::signing::SigningError> {
        let timestamp = clob_server_time().map_err(|r| petal::signing::SigningError {
            code: petal::signing::ErrorCode::Backend,
            message: dispatch_error_message(&r),
        })?;
        Ok((
            eip712_action(
                "polymarket.clob_auth.v1",
                &crate::polymarket::eip712::clob_auth_typed_data(
                    owner,
                    timestamp,
                    CLOB_AUTH_NONCE,
                    chain_id,
                ),
            )?
            .with_spend(petal::signing::Spend::None),
            timestamp,
        ))
    };
    let attempt = match login_attempt(&mut host, wallet, refresh, build) {
        Ok(a) => a,
        Err(e) => return e.into(),
    };
    let timestamp = *attempt.context();
    let action = clob_auth_action_and_hash(owner, timestamp, CLOB_AUTH_NONCE, chain_id);
    let review = serde_json::json!({"operation":"clob_auth", "owner":owner.to_checksum(None),
        "chain_id":chain_id,"timestamp":timestamp,"nonce":CLOB_AUTH_NONCE,"message":action.message,
        "signing_hash":format!("{:#x}",action.signing_hash)});
    if let Err(resp) = store_review_intent(&review_key, &review) {
        return resp;
    }
    let signature = match sign_or_resume(&mut host, wallet, &attempt) {
        Ok(bytes) => {
            match crate::approval::verified_signature(owner, action.signing_hash, &bytes) {
                Ok(signature) => signature.to_string(),
                Err(resp) => return resp,
            }
        }
        Err(halt) => return halt.into(),
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
        return login_failed_after_signing(response);
    }
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
    let _lock = match acquire_trade_lock(wallet, "onboarding-batch") {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
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
pub(crate) mod tests {
    use super::*;

    #[derive(Default)]
    pub(crate) struct Host {
        pub(crate) store: std::collections::HashMap<String, Vec<u8>>,
        outcome: Option<Result<petal::signing::SigningOutcome, petal::signing::SigningError>>,
    }
    impl petal::signing::SigningHost for Host {
        fn package_hash(&self) -> &str {
            "package"
        }
        fn store_get(
            &mut self,
            key: &str,
        ) -> Result<Option<Vec<u8>>, petal::signing::SigningError> {
            Ok(self.store.get(key).cloned())
        }
        fn store_put_new(
            &mut self,
            key: &str,
            bytes: &[u8],
        ) -> Result<bool, petal::signing::SigningError> {
            if self.store.contains_key(key) {
                return Ok(false);
            }
            self.store.insert(key.into(), bytes.to_vec());
            Ok(true)
        }
        fn sign(
            &mut self,
            _: &str,
            _: &petal::signing::PreparedAction,
            _: &str,
        ) -> Result<petal::signing::SigningOutcome, petal::signing::SigningError> {
            self.outcome.clone().unwrap()
        }
        fn status(
            &mut self,
            _: &str,
            _: &str,
        ) -> Result<petal::signing::SigningOutcome, petal::signing::SigningError> {
            self.outcome
                .clone()
                .unwrap_or(Err(petal::signing::SigningError {
                    code: petal::signing::ErrorCode::NotFound,
                    message: "missing".into(),
                }))
        }
        fn now_ms(&mut self) -> Result<u64, petal::signing::SigningError> {
            Ok(100)
        }
    }
    fn login(
        timestamp: u64,
    ) -> Result<(petal::signing::PreparedAction, u64), petal::signing::SigningError> {
        let owner = alloy::primitives::Address::repeat_byte(1);
        let action = eip712_action(
            "polymarket.clob_auth.v1",
            &crate::polymarket::eip712::clob_auth_typed_data(
                owner,
                timestamp,
                CLOB_AUTH_NONCE,
                137,
            ),
        )?
        .with_spend(petal::signing::Spend::None);
        assert_eq!(
            action.payload.hash.as_slice(),
            clob_auth_action_and_hash(owner, timestamp, CLOB_AUTH_NONCE, 137)
                .signing_hash
                .as_slice()
        );
        Ok((action, timestamp))
    }
    #[test]
    fn login_refresh_preserves_pending_and_ambiguous_but_renews_signed() {
        use petal::signing::SigningOutcome;
        let mut host = Host::default();
        let first = login_attempt(&mut host, "test", false, || login(100)).unwrap();
        for outcome in [
            SigningOutcome::ApprovalPending {
                action_id: "a".into(),
                expires_ms: 999,
            },
            SigningOutcome::RecoveryRequired {
                status: "ambiguous".into(),
            },
            SigningOutcome::RecoveryRequired {
                status: "approved_retry".into(),
            },
        ] {
            host.outcome = Some(Ok(outcome));
            let retry = login_attempt(&mut host, "test", true, || login(200)).unwrap();
            assert_eq!(retry.operation_key(), first.operation_key());
            assert_eq!(*retry.context(), 100);
            assert_eq!(host.store.len(), 1);
        }
        host.outcome = Some(Ok(SigningOutcome::Signed {
            signature: vec![1; 65],
        }));
        assert_eq!(
            login_attempt(&mut host, "test", false, || login(200))
                .unwrap()
                .operation_key(),
            first.operation_key()
        );
        let refresh = login_attempt(&mut host, "test", true, || login(200)).unwrap();
        assert_ne!(refresh.operation_key(), first.operation_key());
        assert_eq!(*refresh.context(), 200);
        assert_eq!(host.store.len(), 2);
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
