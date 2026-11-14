use crate::prelude::*;

use crate::polymarket::eip712::{COLLATERAL_ONRAMP, PUSD, USDC_E};
use crate::polymarket::order::parse_micro;
use crate::polymarket::wallet::{approveCall, transferCall, wrapCall};
use crate::polymarket::{Result, validate_wallet_name};
use alloy::primitives::{Address, U256};
use alloy::sol_types::SolCall;
use petal::sdk::{DispatchResponse, EvmTransaction, HostStatus, SdkError};
pub fn create_fund_request(
    wallet: &str,
    account: &petal::WalletAccount,
    body: &[u8],
) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
    }
    let owner = match wallet_address(account) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let req: FundNewRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("fund request JSON: {e}")),
    };
    forget_enso_settings();
    if parse_micro(req.target_pusd.trim()).unwrap_or(0) == 0 {
        return error(-3, "target_pusd must be > 0");
    }
    if !positive_decimal_input(req.max_spend.trim()) {
        return error(-3, "max_spend must be > 0");
    }
    let from_token = match FundingSource::parse(req.from_token.as_deref().unwrap_or("auto")) {
        Ok(source) => source.name().to_string(),
        Err(resp) => return resp,
    };
    let id = next_id(&format!("fund/{wallet}/requests/"), ".json");
    let session = StoreFundSession {
        id: id.clone(),
        wallet: wallet.into(),
        target_pusd: req.target_pusd,
        max_spend: req.max_spend,
        from_token,
        deposit_wallet: deposit.to_checksum(None),
        deposit_wallet_source: "live_factory_resolved".into(),
        status: "draft".into(),
        prepared_funding: None,
        review_intent: None,
        outbox_ids: Vec::new(),
        outbox_inspections: Vec::new(),
        next_transaction: 0,
        plan_md: None,
        approval: None,
        staging_transaction: None,
    };
    store_put_json(
        &format!("fund/{wallet}/requests/{id}.json"),
        &session,
        false,
    )
}

pub fn confirm_fund_request(
    wallet: &str,
    account: &petal::WalletAccount,
    id: &str,
    body: &[u8],
) -> DispatchResponse {
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
    }
    let (chain, _) = match crate::runtime_config::chain() {
        Ok(chain) => chain,
        Err(err) => return error(-4, err),
    };
    let confirmation = match fund_confirmation(body) {
        Ok(confirmation) if confirmation.confirm => confirmation,
        Ok(_) | Err(()) => {
            return error(
                -3,
                "fund confirm requires confirm, y, or {\"confirm\":true}",
            );
        }
    };
    let _lock = match acquire_fund_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let key = format!("fund/{wallet}/requests/{id}.json");
    let mut session = match load_fund_session(wallet, id) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    if session.staging_transaction == Some(session.next_transaction)
        && session.outbox_ids.len() == session.next_transaction
    {
        return error(
            -4,
            "a prior outbox stage may have succeeded without its id becoming durable; refusing to restage automatically",
        );
    }
    let prepared = match session.prepared_funding.clone() {
        Some(prepared) => prepared,
        None => match prepare_funding(wallet, account, &session) {
            Ok(prepared) => {
                session.review_intent = Some(prepared.review_intent.clone());
                session.prepared_funding = Some(prepared);
                session.status = "prepared".into();
                return store_put_json(&key, &session, false);
            }
            Err(resp) => return resp,
        },
    };
    if session.next_transaction >= prepared.transactions.len() {
        return store_put_json(&key, &session, false);
    }
    if let Some(outbox_id) = session.outbox_ids.get(session.next_transaction).cloned() {
        let inspection = match petal::sdk::tx_inspect(wallet, &chain, &outbox_id) {
            Ok(inspection) => inspection,
            Err(err) => return sdk_error(err),
        };
        let record = serde_json::json!({
            "outbox_id": inspection.outbox_id,
            "state": inspection.state,
            "tx_hash": inspection.tx_hash,
            "receipt": inspection.receipt_json.as_deref().and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
        });
        if let Some(existing) = session
            .outbox_inspections
            .iter_mut()
            .find(|value| value.get("outbox_id") == record.get("outbox_id"))
        {
            *existing = record;
        } else {
            session.outbox_inspections.push(record);
        }
        match inspection.state.as_str() {
            "success" => {
                session.next_transaction += 1;
                session.approval = None;
                session.status = if session.next_transaction == prepared.transactions.len() {
                    "complete"
                } else {
                    "transaction_confirmed"
                }
                .into();
                return store_put_json(&key, &session, false);
            }
            "reverted" | "failed" | "cancelled" => {
                session.status = format!("transaction_{}", inspection.state);
                let _ = store_put_json(&key, &session, false);
                return error(-4, "funding transaction failed; refusing automatic retry");
            }
            "sent" => {
                session.status = "awaiting_confirmation".into();
                return store_put_json(&key, &session, false);
            }
            _ => {}
        }
    }
    let transaction = &prepared.transactions[session.next_transaction];
    if session.outbox_ids.len() == session.next_transaction {
        session.staging_transaction = Some(session.next_transaction);
        session.status = "staging_started".into();
        if let DispatchResponse::Error { .. } = store_put_json(&key, &session, false) {
            return error(-4, "failed to persist outbox staging marker");
        }
        let staged = match petal::sdk::tx_stage(&EvmTransaction {
            wallet: wallet.into(),
            chain: chain.clone(),
            to: transaction.to.clone(),
            value_wei: transaction.value_wei.clone(),
            data_hex: transaction.data_hex.clone(),
            nonce: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
        }) {
            Ok(staged) => staged,
            Err(err @ SdkError::Host(HostStatus::Denied | HostStatus::Invalid)) => {
                session.staging_transaction = None;
                session.status = "prepared".into();
                if let DispatchResponse::Error { .. } = store_put_json(&key, &session, false) {
                    return error(-4, "failed to clear rejected outbox staging marker");
                }
                return sdk_error(err);
            }
            Err(err) => return sdk_error(err),
        };
        session.outbox_ids.push(staged.outbox_id);
        session.staging_transaction = None;
        session.plan_md = Some(staged.plan_md);
        session.status = "staged".into();
        if let DispatchResponse::Error { .. } = store_put_json(&key, &session, false) {
            return error(
                -4,
                "staged outbox id could not be persisted; refusing restaging",
            );
        }
        // The authoritative outbox plan (including simulation and warnings) is
        // now readable. Require a distinct write before confirmation.
        return DispatchResponse::Write;
    }
    let outbox_id = &session.outbox_ids[session.next_transaction];
    let outcome = match petal::sdk::tx_confirm(
        wallet,
        &chain,
        outbox_id,
        confirmation.acknowledge_warnings,
    ) {
        Ok(outcome) => outcome,
        Err(err) => return sdk_error(err),
    };
    session.status = if outcome.approval.is_some() {
        "approval_required"
    } else {
        "awaiting_confirmation"
    }
    .into();
    session.plan_md = Some(outcome.plan_md);
    session.approval = outcome.approval.map(|approval| {
        serde_json::json!({
            "action_id": approval.action_id,
            "expires_ms": approval.expires_ms,
            "prepared_artifact_digest": prepared.digest(),
            "retry_state": "approval_required",
            "operation": "fund",
        })
    });
    store_put_json(&key, &session, false)
}

pub fn read_review(wallet: &str, id: &str) -> DispatchResponse {
    match load_fund_session(wallet, id) {
        Ok(session) => match session.review_intent {
            Some(review) => petal::read_json_value(&review),
            None => error(-1, "funding transaction has not been prepared"),
        },
        Err(resp) => resp,
    }
}

pub fn read_approval(wallet: &str, id: &str) -> DispatchResponse {
    match load_fund_session(wallet, id) {
        Ok(session) => match session.approval {
            Some(approval) => petal::read_json_value(&approval),
            None => error(-1, "no funding approval is pending"),
        },
        Err(resp) => resp,
    }
}

/// Where the pUSD comes from. Polymarket mints pUSD 1:1 from USDC.e through
/// its Collateral Onramp, so no swap or quote is needed. Any other token has
/// to become USDC.e first, for example through the Enso Petal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FundingSource {
    /// pUSD if the owner holds enough, otherwise USDC.e.
    Auto,
    Pusd,
    UsdcE,
}

impl FundingSource {
    fn parse(value: &str) -> Result<Self, DispatchResponse> {
        let value = value.trim();
        match value.to_ascii_lowercase().as_str() {
            "auto" => return Ok(Self::Auto),
            "pusd" => return Ok(Self::Pusd),
            "usdc.e" | "usdce" => return Ok(Self::UsdcE),
            _ => {}
        }
        match value.parse::<Address>() {
            Ok(address) if address == PUSD => Ok(Self::Pusd),
            Ok(address) if address == USDC_E => Ok(Self::UsdcE),
            _ => Err(error(
                -3,
                format!(
                    "from_token must be pUSD or USDC.e; Polymarket mints pUSD 1:1 from USDC.e. \
                     Swap other tokens to USDC.e ({}) on Polygon first, for example with the Enso Petal",
                    USDC_E.to_checksum(None)
                ),
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Pusd => "pUSD",
            Self::UsdcE => "USDC.e",
        }
    }

    fn token(self) -> Address {
        match self {
            Self::UsdcE => USDC_E,
            Self::Auto | Self::Pusd => PUSD,
        }
    }
}

fn prepare_funding(
    wallet: &str,
    account: &petal::WalletAccount,
    session: &StoreFundSession,
) -> Result<PreparedFunding, DispatchResponse> {
    let (chain, _) = crate::runtime_config::chain().map_err(|err| error(-4, err))?;
    let owner = wallet_address(account)?;
    let deposit = session
        .deposit_wallet
        .parse::<Address>()
        .map_err(|err| error(-4, format!("corrupt deposit wallet: {err}")))?;
    let target = parse_micro(&session.target_pusd)
        .map(U256::from)
        .map_err(polymarket_error)?;
    let current = read_chain_erc20_balance(PUSD, deposit)?;
    let missing = target.saturating_sub(current);
    if missing.is_zero() {
        return Err(error(-3, "deposit wallet already meets the pUSD target"));
    }
    // pUSD and USDC.e both have 6 decimals and convert 1:1.
    let max_spend = parse_decimal_units(&session.max_spend, 6)?;
    let (source, balance) = match FundingSource::parse(&session.from_token)? {
        FundingSource::Auto => match read_chain_erc20_balance(PUSD, owner)? {
            pusd if pusd >= missing => (FundingSource::Pusd, pusd),
            _ => (
                FundingSource::UsdcE,
                read_chain_erc20_balance(USDC_E, owner)?,
            ),
        },
        source => (source, read_chain_erc20_balance(source.token(), owner)?),
    };
    validate_funding(source, missing, max_spend, balance)?;
    let transactions = match source {
        FundingSource::UsdcE => wrap_transactions(
            deposit,
            missing,
            read_chain_erc20_allowance(USDC_E, owner, COLLATERAL_ONRAMP)?,
        ),
        _ => vec![PreparedEvmTransaction {
            purpose: "direct_pusd_transfer".into(),
            to: PUSD.to_checksum(None),
            value_wei: "0".into(),
            data_hex: erc20_transfer_calldata(deposit, missing),
        }],
    };
    Ok(PreparedFunding {
        review_intent: serde_json::json!({
            "operation": "polymarket_fund",
            "wallet": wallet,
            "chain": chain,
            "from_token": source.name(),
            "recipient": deposit.to_checksum(None),
            "amount_pusd_micro": missing.to_string(),
            "max_spend": session.max_spend,
            "transactions": transactions.clone(),
        }),
        transactions,
    })
}

/// An exact USDC.e approval for the onramp when the current allowance is too
/// small, then `wrap`, which mints pUSD straight into the deposit wallet.
fn wrap_transactions(
    deposit: Address,
    amount: U256,
    allowance: U256,
) -> Vec<PreparedEvmTransaction> {
    let mut transactions = Vec::new();
    if allowance < amount {
        transactions.push(PreparedEvmTransaction {
            purpose: "usdce_exact_approval".into(),
            to: USDC_E.to_checksum(None),
            value_wei: "0".into(),
            data_hex: erc20_approve_calldata(COLLATERAL_ONRAMP, amount),
        });
    }
    transactions.push(PreparedEvmTransaction {
        purpose: "usdce_wrap_to_pusd".into(),
        to: COLLATERAL_ONRAMP.to_checksum(None),
        value_wei: "0".into(),
        data_hex: alloy::primitives::hex::encode_prefixed(
            wrapCall {
                _asset: USDC_E,
                _to: deposit,
                _amount: amount,
            }
            .abi_encode(),
        ),
    });
    transactions
}

fn erc20_approve_calldata(spender: Address, amount: U256) -> String {
    alloy::primitives::hex::encode_prefixed(approveCall { spender, amount }.abi_encode())
}

fn erc20_transfer_calldata(to: Address, amount: U256) -> String {
    alloy::primitives::hex::encode_prefixed(transferCall { to, amount }.abi_encode())
}

fn fund_confirmation(body: &[u8]) -> Result<FundConfirmRequest, ()> {
    let Ok(text) = std::str::from_utf8(body) else {
        return Err(());
    };
    if matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "confirm" | "y" | "yes"
    ) {
        return Ok(FundConfirmRequest {
            confirm: true,
            acknowledge_warnings: false,
        });
    }
    serde_json::from_str(text).map_err(|_| ())
}

struct FundLock {
    key: String,
    expected: Vec<u8>,
}

impl Drop for FundLock {
    fn drop(&mut self) {
        let _ = petal::sdk::store_del_if_value(&self.key, &self.expected);
    }
}

fn acquire_fund_lock(wallet: &str, id: &str) -> Result<FundLock, DispatchResponse> {
    let key = format!("fund/{wallet}/requests/{id}.lock");
    for attempt in 0..2 {
        let body = trade_lock_body(wallet, id)?;
        match petal::sdk::store_put_new(&key, &body, false) {
            Ok(()) => {
                return Ok(FundLock {
                    key,
                    expected: body,
                });
            }
            Err(SdkError::Host(HostStatus::Denied)) if attempt == 0 => {
                let Some(stale) = trade_lock_stale_bytes(&key) else {
                    return Err(error(
                        -3,
                        "another funding operation holds this session lock",
                    ));
                };
                match petal::sdk::store_del_if_value(&key, &stale) {
                    Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => continue,
                    Err(SdkError::Host(HostStatus::Denied)) => {
                        return Err(error(-3, "the funding session lock was refreshed"));
                    }
                    Err(err) => return Err(sdk_error(err)),
                }
            }
            Err(SdkError::Host(HostStatus::Denied)) => {
                return Err(error(
                    -3,
                    "another funding operation holds this session lock",
                ));
            }
            Err(err) => return Err(sdk_error(err)),
        }
    }
    Err(error(
        -3,
        "another funding operation holds this session lock",
    ))
}

/// Delete the Enso API key and router an earlier version stored. Nothing
/// reads them now, and no route can change or remove them otherwise.
fn forget_enso_settings() {
    for key in ["creds/enso-api-key", "settings/enso-router"] {
        let _ = petal::sdk::store_del(key);
    }
}

fn validate_funding(
    source: FundingSource,
    missing: U256,
    max_spend: U256,
    owner_balance: U256,
) -> Result<(), DispatchResponse> {
    if missing > max_spend {
        return Err(error(-3, "missing pUSD exceeds max_spend"));
    }
    if owner_balance < missing {
        return Err(error(
            -3,
            format!(
                "owner {} balance is below the {} needed",
                source.name(),
                crate::polymarket::order::format_micro(missing.saturating_to())
            ),
        ));
    }
    Ok(())
}

fn parse_decimal_units(value: &str, decimals: u8) -> Result<U256, DispatchResponse> {
    let (whole, fraction) = value.trim().split_once('.').unwrap_or((value.trim(), ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > decimals as usize
    {
        return Err(error(-3, "amount has invalid decimal precision"));
    }
    let whole = whole
        .parse::<U256>()
        .map_err(|err| error(-3, format!("amount: {err}")))?;
    let mut fraction = fraction.to_string();
    fraction.extend(core::iter::repeat_n(
        '0',
        decimals as usize - fraction.len(),
    ));
    let fraction = if fraction.is_empty() {
        U256::ZERO
    } else {
        fraction
            .parse::<U256>()
            .map_err(|err| error(-3, format!("amount: {err}")))?
    };
    whole
        .checked_mul(U256::from(10u8).pow(U256::from(decimals)))
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| error(-3, "amount overflows uint256"))
}

fn positive_decimal_input(value: &str) -> bool {
    let mut saw_digit = false;
    let mut saw_nonzero = false;
    let mut saw_dot = false;
    for byte in value.bytes() {
        match byte {
            b'0'..=b'9' => {
                saw_digit = true;
                saw_nonzero |= byte != b'0';
            }
            b'.' if !saw_dot => saw_dot = true,
            _ => return false,
        }
    }
    saw_digit && saw_nonzero && !value.starts_with('.') && !value.ends_with('.')
}

pub fn load_fund_session(wallet: &str, id: &str) -> Result<StoreFundSession, DispatchResponse> {
    if let Err(e) = validate_wallet_name(wallet) {
        return Err(error(-3, e.to_string()));
    }
    let Some(bytes) = store_get(&format!("fund/{wallet}/requests/{id}.json")) else {
        return Err(error(-1, "not found"));
    };
    serde_json::from_slice(&bytes).map_err(|e| error(-4, format!("corrupt fund request: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact bytes the hand-packed encoder produced, so stored funding
    /// requests and their review digests still match.
    #[test]
    fn direct_transfer_calldata_is_canonical() {
        let recipient = Address::repeat_byte(0x11);
        let calldata = erc20_transfer_calldata(recipient, U256::from(7u8));
        assert_eq!(
            calldata,
            format!("0xa9059cbb{:0>64}{:0>64}", "11".repeat(20), "7")
        );
        let decoded = transferCall::abi_decode(&hex::decode(&calldata[2..]).unwrap()).unwrap();
        assert_eq!((decoded.to, decoded.amount), (recipient, U256::from(7u8)));
    }

    /// `forget_enso_settings` reaches the package-wide Enso settings from an
    /// account route only while they stay declared shared.
    #[test]
    fn legacy_enso_settings_stay_declared_shared() {
        let manifest = include_str!("../../petal.toml");
        assert!(manifest.contains(
            r#"shared_keys = ["secrets/creds/enso-api-key", "state/settings/enso-router"]"#
        ));
    }

    #[test]
    fn funding_enforces_spend_cap_and_balance() {
        let missing = U256::from(10_000_000u64);
        for source in [FundingSource::Pusd, FundingSource::UsdcE] {
            assert!(validate_funding(source, missing, U256::from(1_000_000u64), missing).is_err());
            let Err(petal::DispatchResponse::Error { message, .. }) =
                validate_funding(source, missing, missing, U256::from(9_000_000u64))
            else {
                panic!("a short balance must be refused");
            };
            assert!(message.ends_with("below the 10 needed"), "{message}");
            assert!(validate_funding(source, missing, missing, missing).is_ok());
        }
    }

    #[test]
    fn exact_approval_calldata_is_canonical() {
        let spender = Address::repeat_byte(0x22);
        let calldata = erc20_approve_calldata(spender, U256::from(7u8));
        assert_eq!(
            calldata,
            format!("0x095ea7b3{:0>64}{:0>64}", "22".repeat(20), "7")
        );
        let decoded = approveCall::abi_decode(&hex::decode(&calldata[2..]).unwrap()).unwrap();
        assert_eq!(
            (decoded.spender, decoded.amount),
            (spender, U256::from(7u8))
        );
    }

    #[test]
    fn funding_sources_are_pusd_or_usdce_only() {
        let parse = |value: &str| FundingSource::parse(value).ok();
        assert_eq!(parse("auto"), Some(FundingSource::Auto));
        assert_eq!(parse("pUSD"), Some(FundingSource::Pusd));
        assert_eq!(parse("USDC.e"), Some(FundingSource::UsdcE));
        assert_eq!(parse("usdce"), Some(FundingSource::UsdcE));
        assert_eq!(parse(&USDC_E.to_string()), Some(FundingSource::UsdcE));
        assert_eq!(
            parse(&PUSD.to_string().to_lowercase()),
            Some(FundingSource::Pusd)
        );
        // Stored names round-trip, so a saved request prepares the same way.
        for source in [
            FundingSource::Auto,
            FundingSource::Pusd,
            FundingSource::UsdcE,
        ] {
            assert_eq!(parse(source.name()), Some(source));
        }
        // Native POL, native USDC (paused on the onramp) and other tokens need
        // a swap to USDC.e first.
        for refused in [
            "native",
            "pol",
            "usdc",
            "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359",
            "",
        ] {
            assert_eq!(parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn usdce_wraps_into_the_deposit_wallet_with_an_exact_approval() {
        let deposit = Address::repeat_byte(0x33);
        let amount = U256::from(5_000_000u64);
        let wrap = format!(
            "0x62355638{:0>64}{:0>64}{:0>64}",
            hex::encode(USDC_E),
            hex::encode(deposit),
            format!("{amount:x}")
        );

        let both = wrap_transactions(deposit, amount, amount - U256::from(1u8));
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].to, USDC_E.to_checksum(None));
        assert_eq!(
            both[0].data_hex,
            erc20_approve_calldata(COLLATERAL_ONRAMP, amount)
        );
        assert_eq!(both[1].to, COLLATERAL_ONRAMP.to_checksum(None));
        assert_eq!(
            (both[1].value_wei.as_str(), both[1].data_hex.as_str()),
            ("0", wrap.as_str())
        );

        // An existing allowance skips the approval.
        let wrap_only = wrap_transactions(deposit, amount, amount);
        assert_eq!(wrap_only.len(), 1);
        assert_eq!(wrap_only[0].data_hex, wrap);
    }

    #[test]
    fn funding_confirmation_is_explicit() {
        assert!(fund_confirmation(b"confirm").unwrap().confirm);
        assert!(fund_confirmation(br#"{"confirm":true}"#).unwrap().confirm);
        assert!(fund_confirmation(b"").is_err());
        assert!(!fund_confirmation(br#"{"confirm":false}"#).unwrap().confirm);
        assert!(
            fund_confirmation(br#"{"confirm":true,"acknowledge_warnings":true}"#)
                .unwrap()
                .acknowledge_warnings
        );
    }

    #[test]
    fn decimal_units_are_bounded() {
        assert_eq!(
            parse_decimal_units("1.25", 6).unwrap(),
            U256::from(1_250_000u64)
        );
        assert!(parse_decimal_units("1.0000001", 6).is_err());
        assert!(parse_decimal_units("-1", 6).is_err());
        assert!(positive_decimal_input("0.0000001"));
        assert!(!positive_decimal_input("0.0"));
        assert!(!positive_decimal_input("1."));
    }
}
