use crate::*;

use crate::bloom_petal_sdk::DispatchResponse;
use crate::polymarket::{Result, validate_wallet_name};
pub(crate) fn trade_draft_base(wallet: &str, id: &str) -> String {
    format!("trade/{wallet}/drafts/{id}")
}

pub(crate) fn trade_draft_order_key(wallet: &str, id: &str) -> String {
    format!("{}/order.json", trade_draft_base(wallet, id))
}

pub(crate) fn trade_draft_policy_check_key(wallet: &str, id: &str) -> String {
    format!("{}/policy_check.json", trade_draft_base(wallet, id))
}

pub(crate) fn trade_draft_quote_key(wallet: &str, id: &str) -> String {
    format!("{}/quote.json", trade_draft_base(wallet, id))
}

pub(crate) fn trade_draft_review_intent_key(wallet: &str, id: &str) -> String {
    format!("{}/review_intent.json", trade_draft_base(wallet, id))
}

pub(crate) fn trade_draft_post_attempt_key(wallet: &str, id: &str) -> String {
    format!("{}/post_attempt.json", trade_draft_base(wallet, id))
}

pub(crate) fn trade_receipt_key(wallet: &str, id: &str) -> String {
    format!("trade/{wallet}/receipts/{id}/receipt.json")
}

pub(crate) fn trade_revalidate_hint() -> DispatchResponse {
    DispatchResponse::Read(TRADE_REVALIDATE_HINT.into())
}

pub(crate) fn trade_post_hint() -> DispatchResponse {
    DispatchResponse::Read(TRADE_POST_HINT.into())
}

pub(crate) fn trade_cancel_hint() -> DispatchResponse {
    DispatchResponse::Read(TRADE_CANCEL_HINT.into())
}

pub(crate) fn trade_new_hint() -> DispatchResponse {
    DispatchResponse::Read(TRADE_NEW_HINT.into())
}

pub(crate) fn fund_new_hint() -> DispatchResponse {
    DispatchResponse::Read(FUND_NEW_HINT.into())
}

pub(crate) fn load_trade_draft(
    wallet: &str,
    id: &str,
) -> Result<StoreTradeDraft, DispatchResponse> {
    if let Err(e) = validate_wallet_name(wallet) {
        return Err(error(-3, e.to_string()));
    }
    let Some(bytes) = store_get(&trade_draft_order_key(wallet, id)) else {
        return Err(error(-1, "draft not found"));
    };
    let draft: StoreTradeDraft =
        serde_json::from_slice(&bytes).map_err(|e| error(-4, format!("corrupt draft: {e}")))?;
    if draft.wallet != wallet || draft.id != id {
        return Err(error(-4, "draft identity mismatch"));
    }
    Ok(draft)
}
