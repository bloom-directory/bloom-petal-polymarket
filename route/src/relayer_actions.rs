use alloy::primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

use crate::polymarket::claim::{BatchCall, PreparedBatch};
use crate::polymarket::eip712::{Call, FACTORY, PUSD};
use crate::polymarket::signing::wallet_batch_action_and_hash;
use crate::polymarket::wallet::{redeem_positions_call, revoke_calls, transfer_amount_call};
use crate::polymarket::{Market, validate_wallet_name};
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

pub fn redeem_plan(wallet: &str, slug: &str) -> DispatchResponse {
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
    }
    let market: Market = match get_json(&format!(
        "{}/markets/slug/{slug}",
        crate::runtime_config::gamma_url()
    )) {
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

pub fn revoke_plan(wallet: &str) -> DispatchResponse {
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
    }
    DispatchResponse::Read(format!(
        "# Revoke Polymarket approvals\n\nWallet: {wallet}\n\nThis revokes all pUSD allowances and CTF operator approvals through one persisted deposit-wallet batch.\n"
    ).into_bytes())
}

pub fn withdraw_plan(wallet: &str, account: &petal::WalletAccount) -> DispatchResponse {
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
    }
    let owner = match wallet_address(account) {
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

pub fn confirm_redeem(
    ctx: &petal::Ctx,
    wallet: &str,
    account: &petal::WalletAccount,
    slug: &str,
    body: &[u8],
) -> DispatchResponse {
    execute(ctx, RelayerAction::Redeem { slug }, wallet, account, body)
}

pub fn confirm_revoke(
    ctx: &petal::Ctx,
    wallet: &str,
    account: &petal::WalletAccount,
    body: &[u8],
) -> DispatchResponse {
    execute(ctx, RelayerAction::RevokeApprovals, wallet, account, body)
}

pub fn confirm_withdraw(
    ctx: &petal::Ctx,
    wallet: &str,
    account: &petal::WalletAccount,
    body: &[u8],
) -> DispatchResponse {
    execute(ctx, RelayerAction::WithdrawPusd, wallet, account, body)
}

fn execute(
    ctx: &petal::Ctx,
    action: RelayerAction<'_>,
    wallet: &str,
    account: &petal::WalletAccount,
    body: &[u8],
) -> DispatchResponse {
    if let Err(err) = validate_wallet_name(wallet) {
        return error(-3, err.to_string());
    }
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
    }
    let (chain, chain_id) = match crate::runtime_config::chain() {
        Ok(chain) => chain,
        Err(err) => return error(-4, err),
    };
    if !confirmation_body(body) {
        return error(
            -3,
            "confirmation requires confirm, y, or {\"confirm\":true}",
        );
    }
    let owner = match wallet_address(account) {
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
            let built = wallet_batch_action_and_hash(deposit, chain_id, nonce, deadline, &calls);
            let batch_calls: Vec<BatchCall> = calls.iter().map(BatchCall::from).collect();
            let review = serde_json::json!({
                "operation": action.operation(),
                "owner": owner.to_checksum(None),
                "deposit_wallet": deposit.to_checksum(None),
                "chain_id": chain_id,
                "nonce": nonce,
                "deadline": deadline,
                "calls": batch_calls,
                "signing_hash": format!("{:#x}", built.signing_hash),
            });
            let review_hash = match store_review_intent(&review_key, &review) {
                Ok(hash) => hash,
                Err(resp) => return resp,
            };
            let preimage = PreparedBatch {
                deposit_wallet: deposit,
                chain_id,
                nonce,
                deadline,
                calls: batch_calls,
                review_intent_hash: review_hash,
            }
            .to_preimage();
            let preimage = match preimage {
                Ok(preimage) => preimage,
                Err(e) => return polymarket_error(e),
            };
            let prepared = PreparedSigning::new(
                action.operation(),
                "polymarket.relayer_batch",
                owner,
                built.signing_preimage,
                built.signing_hash,
                preimage,
            );
            if let Err(resp) = store_prepared_signing(&prepared_key, &prepared) {
                return resp;
            }
            return DispatchResponse::Write;
        }
        Err(resp) => return resp,
    };
    let batch = match PreparedBatch::from_preimage(&prepared.preimage) {
        Ok(batch) => batch,
        Err(e) => return polymarket_error(e),
    };
    if batch.chain_id != chain_id {
        return error(
            -4,
            "prepared relayer action does not match configured chain",
        );
    }
    if let Err(resp) = verify_review_intent(&review_key, &batch.review_intent_hash) {
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
                if let DispatchResponse::Error { .. } =
                    store_put_json(&receipt_key, &receipt, false)
                {
                    return error(-4, "failed to persist relayer action receipt");
                }
                for key in [&prepared_key, &review_key, &approval_key, &progress_key] {
                    match petal::sdk::store_del(key) {
                        Ok(())
                        | Err(petal::sdk::SdkError::Host(petal::sdk::HostStatus::NotFound)) => {}
                        Err(err) => return sdk_error(err),
                    }
                }
                return DispatchResponse::Write;
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

    let effects = match prepared.hash().and_then(|hash| {
        crate::polymarket::claim::prepared_batch_effects(&chain, &batch, hash)
            .map_err(polymarket_error)
    }) {
        Ok(effects) => effects,
        Err(resp) => return resp,
    };
    // alloy's `Signature` renders v as 27/28, the form the relayer expects.
    let signature = match sign_prepared(ctx, wallet, &prepared, &effects, &approval_key) {
        Ok(signature) => signature.to_string(),
        Err(resp) => return resp,
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
    let tx = match relayer_submit_configured(
        wallet,
        owner,
        &creds,
        serde_json::json!({
            "type": "WALLET",
            "from": owner.to_checksum(None),
            "to": FACTORY.to_checksum(None),
            "nonce": batch.nonce.to_string(),
            "signature": signature,
            "depositWalletParams": {
                "depositWallet": deposit.to_checksum(None),
                "deadline": batch.deadline.to_string(),
                "calls": batch.calls,
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
            let market: Market = get_json(&format!(
                "{}/markets/slug/{slug}",
                crate::runtime_config::gamma_url()
            ))?;
            let condition = market
                .condition_id
                .parse::<B256>()
                .map_err(|err| error(-3, format!("market condition id: {err}")))?;
            Ok(vec![redeem_positions_call(condition, market.neg_risk)])
        }
        RelayerAction::RevokeApprovals => Ok(revoke_calls()),
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
    let amount = if amount.eq_ignore_ascii_case("all") {
        balance
    } else {
        U256::from(crate::polymarket::order::parse_micro(amount).map_err(polymarket_error)?)
    };
    if amount.is_zero() {
        // A zero transfer moves nothing, and its claim could not declare a debit.
        return Err(error(-3, "nothing to withdraw: the amount is zero"));
    }
    if amount > balance {
        return Err(error(-3, "withdraw amount exceeds deposit pUSD balance"));
    }
    Ok(amount)
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
        assert!(withdraw_amount(br#"{"confirm":true,"amount":"0"}"#, balance).is_err());
        assert!(withdraw_amount(br#"{"confirm":true,"amount":"all"}"#, U256::ZERO).is_err());
    }

    #[test]
    fn relayer_confirmation_is_explicit() {
        assert!(confirmation_body(b"confirm"));
        assert!(confirmation_body(br#"{"confirm":true}"#));
        assert!(!confirmation_body(br#"{"confirm":false}"#));
    }
}
