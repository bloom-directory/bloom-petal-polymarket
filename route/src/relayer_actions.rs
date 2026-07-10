use alloy::primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

use crate::polymarket::eip712::{Call, FACTORY, PUSD};
use crate::polymarket::signing::wallet_batch_action_and_hash;
use crate::polymarket::wallet::{redeem_positions_call, transfer_amount_call, v2_revoke_calls};
use crate::polymarket::{Market, POLYGON, validate_wallet_name};
use crate::prelude::*;
use petal::sdk::DispatchResponse;

#[derive(Debug, Clone, Copy)]
enum RelayerAction<'a> {
    Redeem { slug: &'a str },
    RevokeApprovals,
    WithdrawPusd,
}

impl RelayerAction<'_> {
    fn operation(self) -> &'static str {
        match self {
            Self::Redeem { .. } => "redeem",
            Self::RevokeApprovals => "revoke-approvals",
            Self::WithdrawPusd => "withdraw-pusd",
        }
    }

    fn base(self, wallet: &str) -> String {
        match self {
            Self::Redeem { slug } => format!("actions/{wallet}/redeem/{slug}"),
            Self::RevokeApprovals => format!("actions/{wallet}/revoke-approvals"),
            Self::WithdrawPusd => format!("actions/{wallet}/withdraw-pusd"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RelayerProgress {
    prepared_digest: String,
    phase: String,
    transaction_id: Option<String>,
    relayer_state: Option<String>,
}

pub(crate) fn redeem_plan(wallet: &str, slug: &str) -> DispatchResponse {
    let market: Market = match get_json(&format!("{GAMMA}/markets/slug/{slug}")) {
        Ok(market) => market,
        Err(resp) => return resp,
    };
    if market.condition_id.parse::<B256>().is_err() || !market.is_binary() {
        return error(
            -3,
            "redeem requires a binary market with a valid condition id",
        );
    }
    DispatchResponse::Read(format!(
        "# Redeem {slug}\n\nWallet: {wallet}\nCondition: {}\nNeg risk: {}\n\nConfirmation signs and submits the exact persisted deposit-wallet batch.\n",
        market.condition_id, market.neg_risk
    ).into_bytes())
}

pub(crate) fn revoke_plan(wallet: &str) -> DispatchResponse {
    DispatchResponse::Read(format!(
        "# Revoke Polymarket approvals\n\nWallet: {wallet}\n\nThis revokes all pUSD allowances and CTF operator approvals through one persisted deposit-wallet batch.\n"
    ).into_bytes())
}

pub(crate) fn withdraw_plan(wallet: &str) -> DispatchResponse {
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let balance = match read_chain_erc20_balance(PUSD, deposit) {
        Ok(balance) => balance,
        Err(resp) => return resp,
    };
    DispatchResponse::Read(format!(
        "# Withdraw pUSD\n\nWallet: {wallet}\nDeposit wallet: {}\nCurrent raw balance: {balance}\n\nWrite {{\"confirm\":true,\"amount\":\"all\"}} to prepare and submit the exact persisted transfer batch.\n",
        deposit.to_checksum(None)
    ).into_bytes())
}

pub(crate) fn confirm_redeem(wallet: &str, slug: &str, body: &[u8]) -> DispatchResponse {
    execute(RelayerAction::Redeem { slug }, wallet, body)
}

pub(crate) fn confirm_revoke(wallet: &str, body: &[u8]) -> DispatchResponse {
    execute(RelayerAction::RevokeApprovals, wallet, body)
}

pub(crate) fn confirm_withdraw(wallet: &str, body: &[u8]) -> DispatchResponse {
    execute(RelayerAction::WithdrawPusd, wallet, body)
}

fn execute(action: RelayerAction<'_>, wallet: &str, body: &[u8]) -> DispatchResponse {
    if let Err(err) = validate_wallet_name(wallet) {
        return error(-3, err.to_string());
    }
    if !confirmation_body(body) {
        return error(
            -3,
            "confirmation requires confirm, y, or {\"confirm\":true}",
        );
    }
    let owner = match wallet_address(wallet) {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let base = action.base(wallet);
    let prepared_key = format!("{base}/prepared_signing.json");
    let review_key = format!("{base}/review_intent.json");
    let approval_key = format!("{base}/approval.json");
    let progress_key = format!("{base}/progress.json");
    let receipt_key = format!("{base}/receipt.json");

    let prepared = match load_prepared_signing(&prepared_key) {
        Ok(Some(prepared)) => prepared,
        Ok(None) => {
            let calls = match calls_for(action, owner, deposit, body) {
                Ok(calls) => calls,
                Err(resp) => return resp,
            };
            let nonce = match relayer_wallet_nonce(owner) {
                Ok(nonce) => nonce,
                Err(resp) => return resp,
            };
            let deadline = now_secs().saturating_add(BATCH_DEADLINE_SECS);
            let built = wallet_batch_action_and_hash(deposit, POLYGON, nonce, deadline, &calls);
            let calls_json: Vec<_> = calls.iter().map(call_json).collect();
            let review = serde_json::json!({
                "operation": action.operation(),
                "owner": owner.to_checksum(None),
                "deposit_wallet": deposit.to_checksum(None),
                "chain_id": POLYGON,
                "nonce": nonce,
                "deadline": deadline,
                "calls": calls_json,
                "signing_hash": format!("{:#x}", built.signing_hash),
            });
            let review_hash = match store_review_intent(&review_key, &review) {
                Ok(hash) => hash,
                Err(resp) => return resp,
            };
            let prepared = PreparedSigning::new(
                action.operation(),
                "polymarket.relayer_batch",
                owner,
                built.signing_hash,
                serde_json::json!({
                    "deposit_wallet": deposit.to_checksum(None),
                    "nonce": nonce,
                    "deadline": deadline,
                    "calls": calls_json,
                    "review_intent_hash": review_hash,
                }),
            );
            if let Err(resp) = store_prepared_signing(&prepared_key, &prepared) {
                return resp;
            }
            return DispatchResponse::Write;
        }
        Err(resp) => return resp,
    };
    let expected_review = match prepared
        .preimage
        .get("review_intent_hash")
        .and_then(serde_json::Value::as_str)
    {
        Some(hash) => hash,
        None => return error(-4, "prepared relayer batch is missing review hash"),
    };
    if let Err(resp) = verify_review_intent(&review_key, expected_review) {
        return resp;
    }
    let digest = match serde_json::to_vec(&prepared) {
        Ok(bytes) => blake3_hex(&bytes),
        Err(err) => return error(-4, format!("prepared batch JSON: {err}")),
    };
    if let Some(bytes) = store_get(&progress_key) {
        let progress: RelayerProgress = match serde_json::from_slice(&bytes) {
            Ok(progress) => progress,
            Err(err) => return error(-4, format!("corrupt relayer progress: {err}")),
        };
        if progress.prepared_digest != digest {
            return error(-4, "relayer progress does not match prepared batch");
        }
        if let Some(id) = progress.transaction_id {
            let tx = match relayer_transaction(&id) {
                Ok(tx) => tx,
                Err(resp) => return resp,
            };
            let updated = RelayerProgress {
                prepared_digest: digest,
                phase: if tx.is_confirmed() {
                    "confirmed"
                } else if tx.is_failed() {
                    "failed"
                } else {
                    "submitted"
                }
                .into(),
                transaction_id: Some(tx.id.clone()),
                relayer_state: Some(tx.state.clone()),
            };
            if tx.is_confirmed() {
                let receipt = serde_json::json!({
                    "operation": action.operation(),
                    "wallet": wallet,
                    "deposit_wallet": deposit.to_checksum(None),
                    "transaction_id": tx.id,
                    "state": tx.state,
                    "prepared_digest": updated.prepared_digest,
                });
                let _ = store_put_json(&progress_key, &updated, false);
                let _ = petal::sdk::store_del(&approval_key);
                return store_put_json(&receipt_key, &receipt, false);
            }
            let _ = store_put_json(&progress_key, &updated, false);
            return if tx.is_failed() {
                error(-4, "relayer transaction failed")
            } else {
                DispatchResponse::Write
            };
        }
        if progress.phase == "submission_started" {
            return error(
                -4,
                "submission may have succeeded without returning an id; refusing to resubmit",
            );
        }
    }

    let signature = match sign_prepared(wallet, &prepared, &approval_key) {
        Ok(signature) => format!("0x{}", hex::encode(signature)),
        Err(resp) => return resp,
    };
    let calls = match prepared
        .preimage
        .get("calls")
        .and_then(serde_json::Value::as_array)
    {
        Some(calls) => calls,
        None => return error(-4, "prepared relayer batch is missing calls"),
    };
    let nonce = match prepared
        .preimage
        .get("nonce")
        .and_then(serde_json::Value::as_u64)
    {
        Some(value) => value,
        None => return error(-4, "prepared relayer batch is missing nonce"),
    };
    let deadline = match prepared
        .preimage
        .get("deadline")
        .and_then(serde_json::Value::as_u64)
    {
        Some(value) => value,
        None => return error(-4, "prepared relayer batch is missing deadline"),
    };
    let progress = RelayerProgress {
        prepared_digest: digest.clone(),
        phase: "submission_started".into(),
        transaction_id: None,
        relayer_state: None,
    };
    if let DispatchResponse::Error { .. } = store_put_json(&progress_key, &progress, false) {
        return error(-4, "failed to persist relayer submission marker");
    }
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let tx = match relayer_submit_with_builder_repair(
        wallet,
        owner,
        &creds,
        serde_json::json!({
            "type": "WALLET",
            "from": owner.to_checksum(None),
            "to": FACTORY.to_checksum(None),
            "nonce": nonce.to_string(),
            "signature": signature,
            "depositWalletParams": {
                "depositWallet": deposit.to_checksum(None),
                "deadline": deadline.to_string(),
                "calls": calls,
            }
        }),
    ) {
        Ok(tx) => tx,
        Err(resp) => return resp,
    };
    let progress = RelayerProgress {
        prepared_digest: digest,
        phase: "submitted".into(),
        transaction_id: Some(tx.id),
        relayer_state: Some(tx.state),
    };
    store_put_json(&progress_key, &progress, false)
}

fn calls_for(
    action: RelayerAction<'_>,
    owner: Address,
    deposit: Address,
    body: &[u8],
) -> Result<Vec<Call>, DispatchResponse> {
    match action {
        RelayerAction::Redeem { slug } => {
            let market: Market = get_json(&format!("{GAMMA}/markets/slug/{slug}"))?;
            let condition = market
                .condition_id
                .parse::<B256>()
                .map_err(|err| error(-3, format!("market condition id: {err}")))?;
            Ok(vec![redeem_positions_call(condition, market.neg_risk)])
        }
        RelayerAction::RevokeApprovals => Ok(v2_revoke_calls()),
        RelayerAction::WithdrawPusd => {
            let balance = read_chain_erc20_balance(PUSD, deposit)?;
            let amount = withdraw_amount(body, balance)?;
            Ok(vec![transfer_amount_call(PUSD, owner, amount)])
        }
    }
}

fn withdraw_amount(body: &[u8], balance: U256) -> Result<U256, DispatchResponse> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|err| error(-3, format!("withdraw confirmation JSON: {err}")))?;
    let amount = value
        .get("amount")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("all");
    if amount.eq_ignore_ascii_case("all") {
        return Ok(balance);
    }
    let micro = crate::polymarket::order::parse_micro(amount).map_err(polymarket_error)?;
    let amount = U256::from(micro);
    if amount > balance {
        return Err(error(-3, "withdraw amount exceeds deposit pUSD balance"));
    }
    Ok(amount)
}

fn call_json(call: &Call) -> serde_json::Value {
    serde_json::json!({
        "target": call.target.to_checksum(None),
        "value": call.value.to_string(),
        "data": format!("0x{}", hex::encode(call.data.as_ref())),
    })
}

fn confirmation_body(body: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "confirm" | "y" | "yes"
    ) || serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("confirm").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn withdrawal_amount_is_bounded_and_explicit() {
        let balance = U256::from(2_000_000u64);
        assert_eq!(
            withdraw_amount(br#"{"confirm":true,"amount":"all"}"#, balance).unwrap(),
            balance
        );
        assert_eq!(
            withdraw_amount(br#"{"confirm":true,"amount":"1.25"}"#, balance).unwrap(),
            U256::from(1_250_000u64)
        );
        assert!(withdraw_amount(br#"{"confirm":true,"amount":"3"}"#, balance).is_err());
    }

    #[test]
    fn relayer_confirmation_is_explicit() {
        assert!(confirmation_body(b"confirm"));
        assert!(confirmation_body(br#"{"confirm":true}"#));
        assert!(!confirmation_body(br#"{"confirm":false}"#));
    }
}
