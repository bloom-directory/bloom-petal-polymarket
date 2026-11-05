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

trait LoginHost: petal::signing::SigningHost {
    fn set_login_cycle(
        &mut self,
        wallet: &str,
        cycle: u32,
    ) -> Result<(), petal::signing::SigningError>;
}
impl LoginHost for CtxHost<'_> {
    fn set_login_cycle(
        &mut self,
        wallet: &str,
        cycle: u32,
    ) -> Result<(), petal::signing::SigningError> {
        petal::sdk::store_put(
            &format!("onboard/{wallet}/login_cycle.json"),
            cycle.to_string().as_bytes(),
            false,
        )
        .map_err(|e| login_storage_error(e.message()))
    }
}
fn login_storage_error(message: impl Into<String>) -> petal::signing::SigningError {
    petal::signing::SigningError {
        code: petal::signing::ErrorCode::Backend,
        message: message.into(),
    }
}
fn login_cycle(
    host: &mut impl petal::signing::SigningHost,
    wallet: &str,
) -> Result<u32, petal::signing::SigningError> {
    host.store_get(&format!("onboard/{wallet}/login_cycle.json"))?
        .map(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|e| login_storage_error(format!("invalid login cycle: {e}")))
        })
        .transpose()
        .map(|cycle| cycle.unwrap_or(0))
}
fn cycle_scope(wallet: &str, cycle: u32) -> String {
    if cycle == 0 {
        format!("onboard/{wallet}/login")
    } else {
        format!("onboard/{wallet}/login-{cycle}")
    }
}
pub fn login_scope(
    host: &mut impl petal::signing::SigningHost,
    wallet: &str,
) -> Result<String, petal::signing::SigningError> {
    Ok(cycle_scope(wallet, login_cycle(host, wallet)?))
}

/// Called under the onboarding lock. Rollover retains every old attempt and
/// publishes the new scope only after its frozen record exists.
fn login_attempt(
    host: &mut impl LoginHost,
    wallet: &str,
    refresh: bool,
    build: impl Fn() -> Result<(petal::signing::PreparedAction, u64), petal::signing::SigningError>,
) -> Result<Attempt<u64>, petal::signing::SignHalt> {
    let cycle = login_cycle(host, wallet)?;
    let attempt = Attempt::open(host, &cycle_scope(wallet, cycle), &build)?;
    let renew = match attempt.state(host, wallet)? {
        AttemptState::Expired | AttemptState::Cancelled => true,
        AttemptState::Signed | AttemptState::PackageChanged => refresh,
        _ => false,
    };
    if !renew {
        return Ok(attempt);
    }
    if attempt.generation() != 31 {
        return attempt.renew(host, wallet, |_| build());
    }
    if attempt.action().spend != petal::signing::Spend::None {
        return Err(login_storage_error("only a zero-spend login can roll over").into());
    }
    let next = cycle
        .checked_add(1)
        .ok_or_else(|| login_storage_error("login cycle overflow"))?;
    let successor = Attempt::open(host, &cycle_scope(wallet, next), || {
        let (action, timestamp) = build()?;
        if action.spend != petal::signing::Spend::None {
            return Err(login_storage_error("login rollover must remain zero-spend"));
        }
        Ok((action, timestamp))
    })?;
    if successor.action().spend != petal::signing::Spend::None {
        return Err(login_storage_error("stored login rollover is not zero-spend").into());
    }
    host.set_login_cycle(wallet, next)?;
    Ok(successor)
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
        fail_cycle_write: bool,
    }
    impl LoginHost for Host {
        fn set_login_cycle(
            &mut self,
            wallet: &str,
            cycle: u32,
        ) -> Result<(), petal::signing::SigningError> {
            if self.fail_cycle_write {
                return Err(login_storage_error("injected pointer write failure"));
            }
            self.store.insert(
                format!("onboard/{wallet}/login_cycle.json"),
                cycle.to_string().into_bytes(),
            );
            Ok(())
        }
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

    fn last_login_generation() -> Host {
        let mut host = Host::default();
        login_attempt(&mut host, "test", false, || login(100)).unwrap();
        host.outcome = Some(Ok(petal::signing::SigningOutcome::Signed {
            signature: vec![1; 65],
        }));
        for timestamp in 101..132 {
            login_attempt(&mut host, "test", true, || login(timestamp)).unwrap();
        }
        assert_eq!(
            Attempt::<u64>::load(&mut host, "onboard/test/login")
                .unwrap()
                .unwrap()
                .generation(),
            31
        );
        host
    }
    #[test]
    fn login_can_refresh_beyond_one_scope_without_replacing_pending_or_ambiguous() {
        use petal::signing::SigningOutcome;
        let mut host = last_login_generation();
        let original = host.store.clone();
        for outcome in [
            SigningOutcome::ApprovalPending {
                action_id: "pending".into(),
                expires_ms: 999,
            },
            SigningOutcome::RecoveryRequired {
                status: "ambiguous".into(),
            },
        ] {
            host.outcome = Some(Ok(outcome));
            let frozen = login_attempt(&mut host, "test", true, || login(999)).unwrap();
            assert_eq!(frozen.generation(), 31);
            assert_eq!(host.store, original);
        }
        host.outcome = Some(Ok(SigningOutcome::Signed {
            signature: vec![1; 65],
        }));
        for timestamp in 132..171 {
            login_attempt(&mut host, "test", true, || login(timestamp)).unwrap();
        }
        assert_eq!(
            login_scope(&mut host, "test").unwrap(),
            "onboard/test/login-2"
        );
        assert!(host.store.contains_key("onboard/test/login.31.json"));
    }
    #[test]
    fn login_rollover_retries_pointer_failure_without_rebuilding_the_successor() {
        let mut host = last_login_generation();
        host.fail_cycle_write = true;
        assert!(login_attempt(&mut host, "test", true, || login(200)).is_err());
        assert_eq!(
            login_scope(&mut host, "test").unwrap(),
            "onboard/test/login"
        );
        let successor = host.store["onboard/test/login-1.0.json"].clone();
        host.fail_cycle_write = false;
        let retry = login_attempt(&mut host, "test", true, || login(999)).unwrap();
        assert_eq!(*retry.context(), 200);
        assert_eq!(host.store["onboard/test/login-1.0.json"], successor);
        assert_eq!(
            login_scope(&mut host, "test").unwrap(),
            "onboard/test/login-1"
        );
    }
    #[test]
    fn login_rollover_cannot_introduce_spending() {
        let mut host = last_login_generation();
        let original = host.store.clone();
        assert!(
            login_attempt(&mut host, "test", true, || {
                let (mut action, timestamp) = login(200)?;
                action.spend = petal::signing::Spend::Unspecified;
                Ok((action, timestamp))
            })
            .is_err()
        );
        assert_eq!(host.store, original);
        let key = "onboard/test/login.31.json";
        let mut record: serde_json::Value = serde_json::from_slice(&host.store[key]).unwrap();
        record["action"]["spend"] =
            serde_json::to_value(petal::signing::Spend::Unspecified).unwrap();
        host.store
            .insert(key.into(), serde_json::to_vec(&record).unwrap());
        host.outcome = Some(Err(petal::signing::SigningError {
            code: petal::signing::ErrorCode::Expired,
            message: "expired".into(),
        }));
        let before = host.store.clone();
        assert!(login_attempt(&mut host, "test", true, || login(300)).is_err());
        assert_eq!(
            host.store, before,
            "terminal spending attempts cannot roll over"
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
