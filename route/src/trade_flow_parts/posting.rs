use crate::prelude::*;

use crate::polymarket::order::{
    LimitQuote, Order, OrderBody, OrderParams, OrderType, SIG_TYPE_POLY_1271, build_order,
    poly1271_digest, poly1271_signing_preimage, wrap_poly1271_signature,
};
use crate::polymarket::{Result, validate_wallet_name};
use alloy::primitives::{Address, B256, U256};
use petal::sdk::{DispatchResponse, HostStatus, SdkError};
/// Which operation class a post signs under. Broker needs a class to be
/// uniformly fee-bearing or uniformly fee-free, so a builder-coded order
/// cannot share the plain class and the two have their own routes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostClass {
    /// `polymarket.order.poly1271`, fee-free. Refuses a configured code.
    PlainOrder,
    /// `polymarket.builder_code_order.poly1271`, fee-bearing. Requires one.
    BuilderCodeOrder,
}

impl PostClass {
    fn operation_class(self) -> &'static str {
        match self {
            Self::PlainOrder => "polymarket.order.poly1271",
            Self::BuilderCodeOrder => "polymarket.builder_code_order.poly1271",
        }
    }
}

pub fn post_trade_draft(
    ctx: &petal::Ctx,
    wallet: &str,
    account: u32,
    id: &str,
    body: &[u8],
) -> DispatchResponse {
    post_trade_draft_in_class(ctx, wallet, account, id, body, PostClass::PlainOrder)
}

/// Post a draft under the fee-bearing class, declaring the builder fee.
pub fn post_builder_code_trade_draft(
    ctx: &petal::Ctx,
    wallet: &str,
    account: u32,
    id: &str,
    body: &[u8],
) -> DispatchResponse {
    post_trade_draft_in_class(ctx, wallet, account, id, body, PostClass::BuilderCodeOrder)
}

fn post_trade_draft_in_class(
    ctx: &petal::Ctx,
    wallet: &str,
    account: u32,
    id: &str,
    body: &[u8],
    class: PostClass,
) -> DispatchResponse {
    let (_, chain_id) = match crate::runtime_config::chain() {
        Ok(chain) => chain,
        Err(err) => return error(-4, err),
    };
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_segment(id) {
        return error(-3, "invalid draft id");
    }
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
    }
    let req: TradePostRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("post JSON: {e}")),
    };
    if !req.post {
        return error(-3, "post must be true");
    }
    let base = format!("trade/{wallet}/drafts/{id}");
    let _lock = match acquire_trade_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let mut draft = match load_trade_draft(wallet, id) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    if draft.status != "revalidated" && draft.status != "signing_prepared" {
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
    let owner = match wallet_address(wallet, account) {
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
    let policy_warn = policy_check
        .get("policy_warn")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if policy_warn && !req.acknowledge_warnings {
        return error(
            -3,
            "policy warnings require {\"post\":true,\"acknowledge_warnings\":true}",
        );
    }
    let review_intent_bytes =
        match petal::sdk::store_get(&format!("{base}/review_intent.json"), MAX_STORE_BYTES) {
            Ok(bytes) => bytes,
            Err(SdkError::Host(HostStatus::NotFound)) => {
                return error(-3, "missing final review intent; write revalidate first");
            }
            Err(e) => return sdk_error(e),
        };
    let mut review_intent: serde_json::Value = match serde_json::from_slice(&review_intent_bytes) {
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
    let Some(review_object) = review_intent.as_object_mut() else {
        return error(-4, "final review intent is not an object");
    };
    review_object.insert(
        "policy_warnings_acknowledged".into(),
        serde_json::Value::Bool(req.acknowledge_warnings),
    );
    let review_intent_bytes = match serde_json::to_vec_pretty(&review_intent) {
        Ok(bytes) => bytes,
        Err(err) => return error(-4, format!("encode final review intent: {err}")),
    };
    if let Err(err) = petal::sdk::store_put(
        &format!("{base}/review_intent.json"),
        &review_intent_bytes,
        false,
    ) {
        return sdk_error_with_context("persist warning acknowledgement", err);
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
    let prepared_key = format!("{base}/prepared_signing.json");
    let approval_key = format!("{base}/approval.json");
    let (order, prepared, fee_bound) = match load_prepared_signing(&prepared_key) {
        Ok(Some(prepared)) => {
            if prepared.operation != "order" || prepared.owner != owner.to_checksum(None) {
                return error(-4, "prepared order does not match wallet");
            }
            // Both routes share this prepared-signing key, so without this a
            // draft prepared under one class could be signed under the other,
            // declaring a fee the class refuses or omitting one it requires.
            if prepared.intent != class.operation_class() {
                return error(
                    -3,
                    format!(
                        "this draft was prepared to sign under {}; post it through that route, or cancel and re-stage it",
                        prepared.intent
                    ),
                );
            }
            let order = match order_from_prepared(&prepared.preimage) {
                Ok(order) => order,
                Err(resp) => return resp,
            };
            // The declared fee is read back, never recomputed: an operator
            // ceiling change between attempts would otherwise sign a fee the
            // ceremony never showed.
            let bound = match class {
                PostClass::BuilderCodeOrder => {
                    match builder_fee_from_prepared(&prepared.preimage, draft.order_type) {
                        Ok(bound) => Some(bound),
                        Err(resp) => return resp,
                    }
                }
                PostClass::PlainOrder => None,
            };
            (order, prepared, bound)
        }
        Ok(None) => {
            let timestamp_ms = match petal::sdk::try_now_ms() {
                Ok(timestamp_ms) => timestamp_ms,
                Err(err) => return sdk_error_with_context("read order timestamp", err),
            };
            let quote = LimitQuote {
                side: draft.side,
                price_micro: draft.limit_price_micro,
                size_micro: draft.size_micro,
                maker_micro: draft.maker_micro,
                taker_micro: draft.taker_micro,
            };
            let (builder_code, bound) = match class {
                PostClass::PlainOrder => {
                    // A configured code means this draft belongs on the
                    // fee-bearing route, where its fee can be declared.
                    match crate::account_views::resolve_builder_code() {
                        Ok(Some(_)) => {
                            return error(
                                -3,
                                "a builder code is configured, so this order owes a builder fee that polymarket.order.poly1271 cannot declare; post it through `post_builder_code` instead",
                            );
                        }
                        Ok(None) => (None, None),
                        Err(resp) => return resp,
                    }
                }
                PostClass::BuilderCodeOrder => {
                    let attribution = match crate::account_views::resolve_builder_attribution() {
                        Ok(Some(attribution)) => attribution,
                        Ok(None) => {
                            return error(
                                -3,
                                "no builder code is configured, so this route has no fee to declare; write one to settings/builder-code, or post through `post`",
                            );
                        }
                        Err(resp) => return resp,
                    };
                    let bound = match crate::builder_fee::fee_bound(
                        &format!("{:#x}", attribution.code),
                        attribution.ceiling,
                        draft.order_type,
                        &quote,
                    ) {
                        Ok(bound) => bound,
                        Err(err) => return error(-3, format!("builder fee: {err}")),
                    };
                    (Some(attribution.code), Some(bound))
                }
            };
            let order = build_order(
                &OrderParams {
                    token_id,
                    maker: funder,
                    quote,
                    builder_code,
                    signature_type: SIG_TYPE_POLY_1271,
                },
                timestamp_ms,
            );
            let digest = poly1271_digest(&order, chain_id, draft.neg_risk);
            let prepared = PreparedSigning::new(
                "order",
                class.operation_class(),
                owner,
                poly1271_signing_preimage(&order, chain_id, draft.neg_risk),
                digest,
                serde_json::json!({
                    "draft_id": id,
                    "owner": owner.to_checksum(None),
                    "funder": funder.to_checksum(None),
                    "token_id": order.tokenId.to_string(),
                    "salt": order.salt.to_string(),
                    "maker_amount": order.makerAmount.to_string(),
                    "taker_amount": order.takerAmount.to_string(),
                    "side": order.side,
                    "signature_type": order.signatureType,
                    "timestamp_ms": order.timestamp.to_string(),
                    "builder": format!("{:#x}", order.builder),
                    // Bound into the approval, and read back on a retry rather
                    // than recomputed, so a ceiling change cannot sign a fee
                    // the ceremony never showed.
                    "builder_fee_rate_bps": bound.as_ref().map(|b| b.builder_bps),
                    "builder_fee_notional_micro": bound.as_ref().map(|b| b.notional_micro.to_string()),
                    "builder_fee_ceiling_micro": bound.as_ref().map(|b| b.builder_fee_micro.to_string()),
                    "neg_risk": draft.neg_risk,
                    "chain_id": chain_id,
                    "review_intent_hash": review_intent_hash,
                    "policy_warnings_acknowledged": req.acknowledge_warnings,
                }),
            );
            if let Err(resp) = store_prepared_signing(&prepared_key, &prepared) {
                return resp;
            }
            (order, prepared, bound)
        }
        Err(resp) => return resp,
    };
    if prepared
        .preimage
        .get("review_intent_hash")
        .and_then(serde_json::Value::as_str)
        != Some(review_intent_hash.as_str())
    {
        return error(-4, "prepared order does not match final review intent");
    }
    if prepared
        .preimage
        .get("chain_id")
        .and_then(serde_json::Value::as_u64)
        != Some(chain_id)
    {
        return error(-4, "prepared order does not match configured chain");
    }
    let salt = match u64::try_from(order.salt) {
        Ok(salt) => salt,
        Err(_) => return error(-4, "order salt does not fit in u64"),
    };
    draft.salt = Some(salt);
    draft.status = "signing_prepared".into();
    draft.last_error = None;
    let digest = poly1271_digest(&order, chain_id, draft.neg_risk);
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
            "policy_warnings_acknowledged": req.acknowledge_warnings,
            "poly1271_digest_blake3": digest_hash,
            "prepared_ms": now_millis(),
            "status": "signing_prepared"
        }),
        false,
    ) {
        return error(-4, "failed to store signing-prepared post attempt");
    }
    let inner_sig = match &fee_bound {
        Some(bound) => {
            // A market buy is the one order type the sizing actually
            // reduces, so it is the one case the owner must be told about.
            let spend_reduced = matches!(
                draft.order_type,
                crate::polymarket::order::OrderType::FAK | crate::polymarket::order::OrderType::FOK
            ) && draft.side == crate::polymarket::Side::Buy;
            let advisory = crate::builder_fee::builder_code_order_advisory(bound, spend_reduced);
            match crate::approval::sign_prepared_with_fee(
                ctx,
                wallet,
                &prepared,
                &approval_key,
                bound.builder_fee_micro,
                advisory,
            ) {
                Ok(signature) => signature,
                Err(resp) => return resp,
            }
        }
        None => match sign_prepared(ctx, wallet, &prepared, &approval_key) {
            Ok(signature) => signature,
            Err(resp) => return resp,
        },
    };
    let signature = match wrap_poly1271_signature(&order, &inner_sig, chain_id, draft.neg_risk) {
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
                // Recorded so the daily cap measures what left the wallet. A
                // plain order genuinely owes nothing, so zero is truthful.
                fee_microusd: draft.builder_fee_micro.unwrap_or(0),
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
            let _ = petal::sdk::store_del(&prepared_key);
            let _ = petal::sdk::store_del(&approval_key);
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
        Err(DispatchResponse::Error { code: -3, message }) => {
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
                // Recorded so the daily cap measures what left the wallet. A
                // plain order genuinely owes nothing, so zero is truthful.
                fee_microusd: draft.builder_fee_micro.unwrap_or(0),
                limit_price_micro: draft.limit_price_micro,
                size_micro: draft.size_micro,
                salt,
                clob_order_id: None,
                clob_status: "rejected".into(),
                filled_size_micro: None,
                raw_response: serde_json::json!({
                    "error": "CLOB rejected the order",
                    "body_hash": body_hash
                }),
                review_intent_hash: Some(review_intent_hash),
                posted_ms,
            };
            if let DispatchResponse::Error { .. } = store_trade_receipt(wallet, id, &receipt) {
                return error(-4, "failed to persist rejected order receipt");
            }
            draft.status = "rejected".into();
            draft.clob_status = Some("rejected".into());
            draft.last_error = Some(message);
            if let DispatchResponse::Error { .. } =
                store_put_json(&format!("{base}/order.json"), &draft, false)
            {
                return error(-4, "failed to persist rejected order state");
            }
            let _ = petal::sdk::store_del(&prepared_key);
            let _ = petal::sdk::store_del(&approval_key);
            error(-3, "CLOB rejected the order; rejected receipt written")
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
                    // Recorded so the daily cap measures what left the wallet. A
                    // plain order genuinely owes nothing, so zero is truthful.
                    fee_microusd: draft.builder_fee_micro.unwrap_or(0),
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
                // Recorded so the daily cap measures what left the wallet. A
                // plain order genuinely owes nothing, so zero is truthful.
                fee_microusd: draft.builder_fee_micro.unwrap_or(0),
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

/// The fee bound a builder-coded order was prepared with.
///
/// Read back, never recomputed. An operator ceiling change between attempts
/// would otherwise sign a fee the owner's ceremony never showed. The fields
/// are required with no zero default: a zero or absent fee under the
/// fee-bearing class is a lie, not a legacy artifact, because no release ever
/// signed under that class without them. The ceiling is re-derived from the
/// stored notional and rate so a tampered artifact is caught here.
fn builder_fee_from_prepared(
    value: &serde_json::Value,
    order_type: crate::polymarket::order::OrderType,
) -> Result<crate::builder_fee::FeeBound, DispatchResponse> {
    let micro = |key: &str| -> Result<u64, DispatchResponse> {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| error(-4, format!("prepared builder order is missing {key}")))?
            .parse::<u64>()
            .map_err(|err| error(-4, format!("prepared {key} is invalid: {err}")))
    };
    let builder_bps = u16::try_from(
        value
            .get("builder_fee_rate_bps")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| error(-4, "prepared builder order is missing builder_fee_rate_bps"))?,
    )
    .map_err(|_| error(-4, "prepared builder_fee_rate_bps is out of range"))?;
    let notional_micro = micro("builder_fee_notional_micro")?;
    let builder_fee_micro = micro("builder_fee_ceiling_micro")?;
    // A zero rate under the fee-bearing class is the same lie as an absent
    // one: it declares nothing while the order still pays. Only the absent
    // case was checked before.
    if builder_bps == 0 {
        return Err(error(
            -4,
            "prepared builder order declares a zero fee rate, which this class cannot carry",
        ));
    }
    let expected = crate::builder_code::builder_fee_ceiling_micro(notional_micro, builder_bps)
        .map_err(|err| error(-4, format!("prepared builder fee: {err}")))?;
    if expected != builder_fee_micro {
        return Err(error(
            -4,
            "prepared builder fee does not match its stored notional and rate",
        ));
    }
    // Self-consistency is not enough: the three fee fields agreeing with each
    // other proves nothing if the notional is unrelated to what the signed
    // order actually gives. For a buy that is `maker_amount`, in the same
    // preimage, so bind them.
    // Self-consistency is not enough: the three fee fields agreeing with each
    // other proves nothing if the notional is unrelated to the signed order.
    // Bind it per side — a buy's floor is what it gives, a sell's is its shares
    // at par — and require the side to be present rather than skipping the
    // binding when it is missing.
    let side = value
        .get("side")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| error(-4, "prepared builder order is missing side"))?;
    let floor_key = match side {
        0 => "maker_amount",
        1 => "taker_amount",
        other => {
            return Err(error(
                -4,
                format!("prepared builder order has an unknown side {other}"),
            ));
        }
    };
    if notional_micro < micro(floor_key)? {
        return Err(error(
            -4,
            "prepared builder fee was priced on less than the order moves",
        ));
    }
    // Blanking this would show the owner "Builder code  is paid on this
    // order." A fee-bearing artifact without a code is not resumable.
    let code_hex = value
        .get("builder")
        .and_then(serde_json::Value::as_str)
        .filter(|code| !code.is_empty())
        .ok_or_else(|| error(-4, "prepared builder order is missing its builder code"))?
        .to_owned();
    Ok(crate::builder_fee::FeeBound {
        code_hex,
        builder_bps,
        // The stored rate is authoritative; the basis only labels which side
        // it came from, and the draft's order type is stable across attempts.
        basis: crate::builder_fee::FeeRateBasis::for_order_type(order_type),
        notional_micro,
        builder_fee_micro,
    })
}

fn order_from_prepared(value: &serde_json::Value) -> Result<Order, DispatchResponse> {
    let string = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| error(-4, format!("prepared order is missing {name}")))
    };
    let parse_u256 = |name: &str| {
        string(name)?
            .parse::<U256>()
            .map_err(|err| error(-4, format!("prepared order {name} is invalid: {err}")))
    };
    let funder = string("funder")?
        .parse::<Address>()
        .map_err(|err| error(-4, format!("prepared order funder is invalid: {err}")))?;
    Ok(Order {
        salt: parse_u256("salt")?,
        maker: funder,
        signer: funder,
        tokenId: parse_u256("token_id")?,
        makerAmount: parse_u256("maker_amount")?,
        takerAmount: parse_u256("taker_amount")?,
        side: value
            .get("side")
            .and_then(serde_json::Value::as_u64)
            .and_then(|side| u8::try_from(side).ok())
            .ok_or_else(|| error(-4, "prepared order side is invalid"))?,
        signatureType: value
            .get("signature_type")
            .and_then(serde_json::Value::as_u64)
            .and_then(|kind| u8::try_from(kind).ok())
            .ok_or_else(|| error(-4, "prepared order signature type is invalid"))?,
        timestamp: parse_u256("timestamp_ms")?,
        metadata: B256::ZERO,
        // Absent on prepared signings written before builder attribution was
        // wired up; those orders were signed with a zero builder field, so
        // defaulting to zero here keeps them reconstructing byte-identical
        // to what was actually signed.
        builder: match value.get("builder").and_then(serde_json::Value::as_str) {
            Some(text) => text
                .parse::<B256>()
                .map_err(|err| error(-4, format!("prepared order builder is invalid: {err}")))?,
            None => B256::ZERO,
        },
    })
}

pub fn cancel_trade_receipt(wallet: &str, account: u32, id: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    if !is_safe_segment(id) {
        return error(-3, "invalid receipt id");
    }
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
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
    let owner = match wallet_address(wallet, account) {
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

pub fn mark_trade_draft_cancelled(wallet: &str, id: &str) -> Result<(), DispatchResponse> {
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

pub fn discoverable_order_ids(wallet: &str, account: u32) -> Result<Vec<String>, DispatchResponse> {
    if let Err(err) = validate_wallet_name(wallet) {
        return Err(error(-3, err.to_string()));
    }
    let owner = wallet_address(wallet, account)?;
    let creds = load_creds(wallet)?;
    let orders = clob_l2_get_json(owner, &creds, "/data/orders", &[])?;
    let rows = orders
        .as_array()
        .or_else(|| orders.get("data").and_then(serde_json::Value::as_array))
        .or_else(|| orders.get("orders").and_then(serde_json::Value::as_array));
    let mut ids: Vec<_> = rows
        .into_iter()
        .flatten()
        .filter_map(clob_response_order_id)
        .filter(|id| petal::is_safe_segment(id))
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn cancel_discovered_order(
    wallet: &str,
    account: u32,
    order_id: &str,
    body: &[u8],
) -> DispatchResponse {
    if let Err(resp) = require_deposit_wallet_trading() {
        return resp;
    }
    if !petal::is_safe_segment(order_id) {
        return error(-3, "invalid CLOB order id");
    }
    let text = match core::str::from_utf8(body) {
        Ok(text) => text.trim(),
        Err(_) => return error(-3, "cancel body must be UTF-8"),
    };
    let confirmed = matches!(text.to_ascii_lowercase().as_str(), "confirm" | "y" | "yes")
        || serde_json::from_str::<TradeCancelRequest>(text).is_ok_and(|request| request.cancel);
    if !confirmed {
        return error(-3, "cancel must be explicitly confirmed");
    }
    let _lock = match acquire_trade_lock(wallet, order_id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let owner = match wallet_address(wallet, account) {
        Ok(owner) => owner,
        Err(resp) => return resp,
    };
    let creds = match load_creds(wallet) {
        Ok(creds) => creds,
        Err(resp) => return resp,
    };
    let orders = match clob_l2_get_json(owner, &creds, "/data/orders", &[]) {
        Ok(orders) => orders,
        Err(resp) => return resp,
    };
    let discoverable = orders
        .as_array()
        .or_else(|| orders.get("data").and_then(serde_json::Value::as_array))
        .or_else(|| orders.get("orders").and_then(serde_json::Value::as_array))
        .into_iter()
        .flatten()
        .filter_map(clob_response_order_id)
        .any(|id| id == order_id);
    if !discoverable {
        return error(
            -3,
            "order is not discoverable from account orders; refusing cancellation",
        );
    }
    let request = serde_json::json!({"orderID": order_id}).to_string();
    let raw = match clob_l2_delete_json(owner, &creds, "/order", &request) {
        Ok(raw) => raw,
        Err(resp) => return resp,
    };
    if !clob_cancel_confirmed(&raw, order_id) {
        return error(-4, "CLOB response did not confirm cancellation");
    }
    append_trade_audit(
        wallet,
        "discovered_order_cancelled",
        serde_json::json!({"clob_order_id": order_id}),
    )
}

#[cfg(test)]
mod builder_fee_retry_tests {
    use super::*;
    use crate::polymarket::order::OrderType;

    fn preimage(rate: u64, notional: &str, ceiling: &str) -> serde_json::Value {
        serde_json::json!({
            "builder": "0xabc",
            "side": 0,
            "maker_amount": notional,
            "builder_fee_rate_bps": rate,
            "builder_fee_notional_micro": notional,
            "builder_fee_ceiling_micro": ceiling,
        })
    }

    #[test]
    fn a_retry_reads_the_stored_fee_rather_than_recomputing_it() {
        let bound = builder_fee_from_prepared(&preimage(100, "10000000", "100000"), OrderType::FAK)
            .unwrap();
        assert_eq!(bound.builder_bps, 100);
        assert_eq!(bound.notional_micro, 10_000_000);
        assert_eq!(bound.builder_fee_micro, 100_000);
        assert_eq!(bound.total_fee_micro(), 100_000);
    }

    /// A zero or absent fee under the fee-bearing class is a lie, not a legacy
    /// artifact: no release ever signed under that class without these fields.
    #[test]
    fn a_prepared_builder_order_missing_its_fee_fields_is_refused() {
        for key in [
            "builder_fee_rate_bps",
            "builder_fee_notional_micro",
            "builder_fee_ceiling_micro",
        ] {
            let mut value = preimage(100, "10000000", "100000");
            value.as_object_mut().unwrap().remove(key);
            assert!(
                builder_fee_from_prepared(&value, OrderType::FAK).is_err(),
                "{key} must be required"
            );
        }
    }

    #[test]
    fn a_tampered_fee_that_disagrees_with_its_notional_is_refused() {
        // 10 pUSD at 100 bps is 0.1 pUSD; claiming less is caught.
        assert!(
            builder_fee_from_prepared(&preimage(100, "10000000", "1"), OrderType::FAK).is_err()
        );
    }

    /// A zero rate and a notional unrelated to the signed amount both passed
    /// the old self-consistency check, which only proved the three stored fee
    /// fields agreed with each other.
    #[test]
    fn a_prepared_builder_order_cannot_declare_a_zero_rate_or_an_unrelated_notional() {
        let mut zero = preimage(100, "10000000", "100000");
        zero["builder_fee_rate_bps"] = serde_json::json!(0);
        zero["builder_fee_ceiling_micro"] = serde_json::json!("0");
        assert!(
            builder_fee_from_prepared(&zero, OrderType::FAK).is_err(),
            "a zero rate declares nothing while the order still pays"
        );

        // Internally consistent, but priced on a tenth of what the order gives.
        let mut understated = preimage(100, "1000000", "10000");
        understated["maker_amount"] = serde_json::json!("10000000");
        assert!(
            builder_fee_from_prepared(&understated, OrderType::FAK).is_err(),
            "the notional must be bound to what the signed order gives"
        );

        // A sell was previously unbound entirely: the side conjunct made the
        // binding buy-only, so any notional passed.
        let mut sell = preimage(100, "1", "1");
        sell["side"] = serde_json::json!(1);
        sell["taker_amount"] = serde_json::json!("10000000");
        assert!(
            builder_fee_from_prepared(&sell, OrderType::FAK).is_err(),
            "a sell's notional must be bound to what the order moves"
        );

        // An absent side skips nothing now; it is refused.
        let mut sideless = preimage(100, "10000000", "100000");
        sideless.as_object_mut().unwrap().remove("side");
        assert!(builder_fee_from_prepared(&sideless, OrderType::FAK).is_err());

        // A fee-bearing artifact with no builder code cannot be resumed.
        let mut codeless = preimage(100, "10000000", "100000");
        codeless["builder"] = serde_json::json!("");
        assert!(builder_fee_from_prepared(&codeless, OrderType::FAK).is_err());

        // The honest version of the same order is accepted.
        let honest = preimage(100, "10000000", "100000");
        assert!(builder_fee_from_prepared(&honest, OrderType::FAK).is_ok());
    }

    /// Both routes share one prepared-signing key, so the guard compares the
    /// stored intent against the class being posted. Asserting only that the
    /// two class strings differ left the guard itself untested.
    #[test]
    fn a_draft_prepared_under_one_class_is_refused_by_the_other() {
        assert_ne!(
            PostClass::PlainOrder.operation_class(),
            PostClass::BuilderCodeOrder.operation_class()
        );
        assert_eq!(
            PostClass::BuilderCodeOrder.operation_class(),
            "polymarket.builder_code_order.poly1271"
        );
        // The comparison the guard performs, in both directions.
        for (stored, posting) in [
            (PostClass::PlainOrder, PostClass::BuilderCodeOrder),
            (PostClass::BuilderCodeOrder, PostClass::PlainOrder),
        ] {
            assert!(
                stored.operation_class() != posting.operation_class(),
                "a draft stored as {} must not post as {}",
                stored.operation_class(),
                posting.operation_class()
            );
        }
    }
}
