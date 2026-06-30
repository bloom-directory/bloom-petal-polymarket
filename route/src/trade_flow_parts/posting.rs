use crate::*;

use crate::bloom_petal_sdk::{DispatchResponse, HostStatus, SdkError, SignRequest};
use crate::order::{
    LimitQuote, OrderBody, OrderParams, OrderType, SIG_TYPE_POLY_1271, build_order,
    poly1271_digest, wrap_poly1271_signature,
};
use crate::polymarket::{POLYGON, Result, validate_wallet_name};
use alloy::primitives::U256;
pub(crate) fn post_trade_draft(wallet: &str, id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_segment(id) {
        return error(-3, "invalid draft id");
    }
    let req: TradePostRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("post JSON: {e}")),
    };
    if !req.post {
        return error(-3, "post must be true");
    }
    let base = trade_draft_base(wallet, id);
    let _lock = match acquire_trade_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let mut draft = match load_trade_draft(wallet, id) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    if draft.status != "revalidated" {
        return error(
            -3,
            format!(
                "draft {id} is '{}' and cannot be posted; write revalidate first",
                draft.status
            ),
        );
    }
    if draft.order_type == OrderType::GTD {
        return error(-3, "posting GTD orders is pending expiry parity");
    }
    if let Err(resp) = check_geoblock() {
        return resp;
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let funder = match tradeable_deposit_wallet(wallet, owner) {
        Ok(funder) => funder,
        Err(resp) => return resp,
    };
    let (policy_check, sell_preflight) =
        match refresh_trade_post_inputs(wallet, &base, &mut draft, owner) {
            Ok(inputs) => inputs,
            Err(resp) => return resp,
        };
    let review_intent_bytes =
        match bloom_petal_sdk::store_get(&format!("{base}/review_intent.json"), MAX_STORE_BYTES) {
            Ok(bytes) => bytes,
            Err(SdkError::Host(HostStatus::NotFound)) => {
                return error(-3, "missing final review intent; write revalidate first");
            }
            Err(e) => return sdk_error(e),
        };
    let review_intent: serde_json::Value = match serde_json::from_slice(&review_intent_bytes) {
        Ok(value) => value,
        Err(e) => return error(-4, format!("corrupt review intent: {e}")),
    };
    if review_intent
        .get("status")
        .and_then(serde_json::Value::as_str)
        != Some("final_review_staged")
        || review_intent
            .get("posting_enabled")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return error(-3, "final review intent does not enable posting");
    }
    if let Err(message) = review_intent_matches_draft(
        &review_intent,
        &draft,
        owner,
        funder,
        &policy_check,
        sell_preflight.as_ref(),
    ) {
        return error(
            -3,
            format!("{message}; write revalidate again before posting"),
        );
    }
    let review_intent_hash = blake3_hex(&review_intent_bytes);
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let token_id = match draft.token_id.parse::<U256>() {
        Ok(token_id) => token_id,
        Err(e) => return error(-4, format!("token id parse: {e}")),
    };
    let order = build_order(&OrderParams {
        token_id,
        maker: funder,
        quote: LimitQuote {
            side: draft.side,
            price_micro: draft.limit_price_micro,
            size_micro: draft.size_micro,
            maker_micro: draft.maker_micro,
            taker_micro: draft.taker_micro,
        },
        builder_code: None,
        signature_type: SIG_TYPE_POLY_1271,
    });
    let salt = match u64::try_from(order.salt) {
        Ok(salt) => salt,
        Err(_) => return error(-4, "order salt does not fit in u64"),
    };
    draft.salt = Some(salt);
    draft.status = "signing_prepared".into();
    draft.last_error = None;
    let digest = poly1271_digest(&order, POLYGON, draft.neg_risk);
    let digest_hash = blake3_hex(digest.as_slice());
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/order.json"), &draft, false)
    {
        return error(-4, "failed to store signing-prepared draft");
    }
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/post_attempt.json"),
        &serde_json::json!({
            "wallet": wallet,
            "draft_id": id,
            "owner": owner.to_checksum(None),
            "funder": funder.to_checksum(None),
            "salt": salt,
            "review_intent_hash": review_intent_hash.clone(),
            "poly1271_digest_blake3": digest_hash,
            "prepared_ms": now_millis(),
            "status": "signing_prepared"
        }),
        false,
    ) {
        return error(-4, "failed to store signing-prepared post attempt");
    }
    let inner_sig = match bloom_petal_sdk::sign_hash(&SignRequest {
        wallet: wallet.into(),
        hash32: digest.into(),
        purpose: "polymarket.order.poly1271".into(),
    }) {
        Ok(sig) if sig.len() == 65 => sig,
        Ok(sig) => return error(-4, format!("sign_hash returned {} bytes", sig.len())),
        Err(e) => return sdk_error(e),
    };
    let signature = match wrap_poly1271_signature(&order, &inner_sig, POLYGON, draft.neg_risk) {
        Ok(signature) => signature,
        Err(e) => return polymarket_error(e),
    };
    let order_body = match OrderBody::from_signed(&order, &signature, &creds.key, draft.order_type)
    {
        Ok(body) => body,
        Err(e) => return polymarket_error(e),
    };
    let body_str = match serde_json::to_string(&order_body) {
        Ok(body) => body,
        Err(e) => return error(-4, format!("order body json: {e}")),
    };
    let body_hash = blake3_hex(body_str.as_bytes());
    if let DispatchResponse::Error { .. } = store_put_json(
        &format!("{base}/post_attempt.json"),
        &serde_json::json!({
            "wallet": wallet,
            "draft_id": id,
            "owner": owner.to_checksum(None),
            "funder": funder.to_checksum(None),
            "salt": salt,
            "review_intent_hash": review_intent_hash.clone(),
            "order_body_blake3": body_hash.clone(),
            "signed_ms": now_millis(),
            "status": "signed"
        }),
        false,
    ) {
        return error(-4, "failed to store post attempt");
    }
    draft.status = "signed".into();
    if let DispatchResponse::Error { .. } =
        store_put_json(&format!("{base}/order.json"), &draft, false)
    {
        return error(-4, "failed to store signed draft");
    }
    match clob_l2_post_json(owner, &creds, "/order", &body_str) {
        Ok(raw_response) => {
            let status = clob_response_status(&raw_response);
            let clob_order_id = clob_response_order_id(&raw_response);
            let filled_size_micro = clob_response_filled_size_micro(&raw_response);
            let posted_ms = now_millis();
            let receipt = StoreTradeReceipt {
                draft_id: id.into(),
                wallet: wallet.into(),
                slug: draft.slug.clone(),
                token_id: draft.token_id.clone(),
                side: draft.side,
                order_type: draft.order_type,
                funder: Some(funder.to_checksum(None)),
                signature_type: SIG_TYPE_POLY_1271,
                amount_microusd: draft.amount_micro,
                limit_price_micro: draft.limit_price_micro,
                size_micro: draft.size_micro,
                salt,
                clob_order_id: clob_order_id.clone(),
                clob_status: status.clone(),
                filled_size_micro,
                raw_response: clob_response_public_summary(
                    &status,
                    &clob_order_id,
                    filled_size_micro,
                ),
                review_intent_hash: Some(review_intent_hash),
                posted_ms,
            };
            if let DispatchResponse::Error { .. } = store_trade_receipt(wallet, id, &receipt) {
                return error(-4, "failed to store receipt");
            }
            draft.status =
                if clob_status_excluded_from_daily_cap(status.as_str(), Some(draft.order_type)) {
                    "rejected".into()
                } else {
                    "posted".into()
                };
            draft.clob_order_id = clob_order_id;
            draft.clob_status = Some(status);
            draft.last_error = None;
            store_put_json(&format!("{base}/order.json"), &draft, false)
        }
        Err(resp) => {
            if let Some(raw_response) =
                reconcile_ambiguous_post(owner, &creds, &draft, funder, salt)
            {
                let status = clob_response_status(&raw_response);
                let clob_order_id = clob_response_order_id(&raw_response);
                let filled_size_micro = clob_response_filled_size_micro(&raw_response);
                let posted_ms = now_millis();
                let receipt = StoreTradeReceipt {
                    draft_id: id.into(),
                    wallet: wallet.into(),
                    slug: draft.slug.clone(),
                    token_id: draft.token_id.clone(),
                    side: draft.side,
                    order_type: draft.order_type,
                    funder: Some(funder.to_checksum(None)),
                    signature_type: SIG_TYPE_POLY_1271,
                    amount_microusd: draft.amount_micro,
                    limit_price_micro: draft.limit_price_micro,
                    size_micro: draft.size_micro,
                    salt,
                    clob_order_id: clob_order_id.clone(),
                    clob_status: status.clone(),
                    filled_size_micro,
                    raw_response: clob_reconciled_public_summary(
                        &status,
                        &clob_order_id,
                        filled_size_micro,
                    ),
                    review_intent_hash: Some(review_intent_hash),
                    posted_ms,
                };
                if let DispatchResponse::Error { .. } = store_put_json(
                    &format!("{base}/post_attempt.json"),
                    &serde_json::json!({
                        "wallet": wallet,
                        "draft_id": id,
                        "owner": owner.to_checksum(None),
                        "funder": funder.to_checksum(None),
                        "salt": salt,
                        "review_intent_hash": receipt.review_intent_hash.clone(),
                        "order_body_blake3": body_hash,
                        "reconciled_ms": posted_ms,
                        "status": "reconciled_open_order",
                        "clob_order_id": clob_order_id,
                        "clob_status": status
                    }),
                    false,
                ) {
                    return error(-4, "failed to store reconciled post attempt");
                }
                if let DispatchResponse::Error { .. } = store_trade_receipt(wallet, id, &receipt) {
                    return error(-4, "failed to store reconciled receipt");
                }
                draft.status = if clob_status_excluded_from_daily_cap(
                    receipt.clob_status.as_str(),
                    Some(draft.order_type),
                ) {
                    "rejected".into()
                } else {
                    "posted".into()
                };
                draft.clob_order_id = receipt.clob_order_id;
                draft.clob_status = Some(receipt.clob_status);
                draft.last_error = None;
                return store_put_json(&format!("{base}/order.json"), &draft, false);
            }
            let posted_ms = now_millis();
            let receipt = StoreTradeReceipt {
                draft_id: id.into(),
                wallet: wallet.into(),
                slug: draft.slug.clone(),
                token_id: draft.token_id.clone(),
                side: draft.side,
                order_type: draft.order_type,
                funder: Some(funder.to_checksum(None)),
                signature_type: SIG_TYPE_POLY_1271,
                amount_microusd: draft.amount_micro,
                limit_price_micro: draft.limit_price_micro,
                size_micro: draft.size_micro,
                salt,
                clob_order_id: None,
                clob_status: "ambiguous".into(),
                filled_size_micro: None,
                raw_response: serde_json::json!({
                    "error": "post outcome unknown after signing",
                    "body_hash": body_hash
                }),
                review_intent_hash: Some(review_intent_hash),
                posted_ms,
            };
            draft.status = "ambiguous".into();
            draft.clob_status = Some("ambiguous".into());
            draft.last_error = Some("post outcome unknown after signing".into());
            if let DispatchResponse::Error { .. } =
                store_put_json(&format!("{base}/order.json"), &draft, false)
            {
                return error(-4, "post outcome ambiguous and failed to store draft state");
            }
            if let DispatchResponse::Error { .. } = store_trade_receipt(wallet, id, &receipt) {
                return error(
                    -4,
                    "post outcome ambiguous and failed to persist receipt/audit",
                );
            }
            let _ = resp;
            error(
                -4,
                "CLOB post outcome unknown after signing; ambiguous receipt written",
            )
        }
    }
}

pub(crate) fn cancel_trade_receipt(wallet: &str, id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_segment(id) {
        return error(-3, "invalid receipt id");
    }
    let req: TradeCancelRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("cancel JSON: {e}")),
    };
    if !req.cancel {
        return error(-3, "cancel must be true");
    }
    let _lock = match acquire_trade_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let receipt_key = format!("trade/{wallet}/receipts/{id}/receipt.json");
    let Some(bytes) = store_get(&receipt_key) else {
        return error(-1, "receipt not found");
    };
    let mut receipt: StoreTradeReceipt = match serde_json::from_slice(&bytes) {
        Ok(receipt) => receipt,
        Err(e) => return error(-4, format!("corrupt receipt: {e}")),
    };
    if receipt.wallet != wallet || receipt.draft_id != id {
        return error(-4, "receipt identity mismatch");
    }
    if receipt.clob_status == "cancelled" {
        if let Err(resp) = mark_trade_draft_cancelled(wallet, id) {
            return resp;
        }
        return DispatchResponse::Write;
    }
    let Some(order_id) = receipt.clob_order_id.clone() else {
        return error(-3, "receipt has no CLOB order id to cancel");
    };
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let body = serde_json::json!({ "orderID": order_id }).to_string();
    let raw = match clob_l2_delete_json(owner, &creds, "/order", &body) {
        Ok(raw) => raw,
        Err(resp) => return resp,
    };
    if !clob_cancel_confirmed(&raw, &order_id) {
        return error(-4, "CLOB cancel response did not confirm cancellation");
    }
    receipt.clob_status = "cancelled".into();
    receipt.raw_response = serde_json::json!({
        "status": "cancelled",
        "order_id": order_id,
        "response_redacted": true
    });
    if let DispatchResponse::Error { .. } = append_trade_audit(
        wallet,
        "order_cancelled",
        serde_json::json!({
            "draft_id": id,
            "clob_order_id": order_id,
        }),
    ) {
        return error(-4, "failed to write cancel audit");
    }
    if let DispatchResponse::Error { .. } = store_put_json(&receipt_key, &receipt, false) {
        return error(-4, "failed to update receipt");
    }
    if let Err(resp) = mark_trade_draft_cancelled(wallet, id) {
        return resp;
    }
    DispatchResponse::Write
}

pub(crate) fn mark_trade_draft_cancelled(wallet: &str, id: &str) -> Result<(), DispatchResponse> {
    let draft_key = format!("trade/{wallet}/drafts/{id}/order.json");
    if let Some(bytes) = store_get(&draft_key) {
        let mut draft: StoreTradeDraft = match serde_json::from_slice(&bytes) {
            Ok(draft) => draft,
            Err(e) => return Err(error(-4, format!("corrupt draft: {e}"))),
        };
        if draft.wallet != wallet || draft.id != id {
            return Err(error(-4, "draft identity mismatch"));
        }
        draft.status = "cancelled".into();
        draft.clob_status = Some("cancelled".into());
        draft.last_error = None;
        if let DispatchResponse::Error { .. } = store_put_json(&draft_key, &draft, false) {
            return Err(error(-4, "failed to update draft"));
        }
    }
    Ok(())
}
