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
    // Both signing paths pass here, and neither may sign a buy on the
    // fee-bearing class.
    if let Err(resp) = fee_bearing_class_can_bound(class, draft.side) {
        return resp;
    }
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
            if let Err(resp) = class_matches_prepared(&prepared.intent, class) {
                return resp;
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
                    match builder_fee_from_prepared(&prepared.preimage, draft.order_type, &draft) {
                        Ok(bound) => Some(bound),
                        Err(resp) => return resp,
                    }
                }
                PostClass::PlainOrder => {
                    // The fee-free class declares no fee, and its ceremony
                    // shows no advisory and names no beneficiary at all. An
                    // order carrying a code under it is still charged by the
                    // venue and still pays whoever owns that code, and the
                    // owner approves it having been shown nothing.
                    //
                    // The fresh path cannot build one — it passes no code on
                    // this class — so a stored artifact is the only way one
                    // can exist, and the digest check above cannot help: an
                    // artifact and its hash are the same object, so one
                    // written with both fields consistent is self-validating.
                    if let Err(resp) = plain_order_carries_no_code(&order) {
                        return resp;
                    }
                    None
                }
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
                    // A code that charges this order belongs on the
                    // fee-bearing route, where its fee can be declared. A
                    // ceiling legitimately zero on the side this order pays
                    // charges nothing, and the fee-bearing route refuses to
                    // declare a zero fee, so refusing here too left a
                    // maker-only builder unable to place any FAK or FOK order
                    // through either route. Such an order posts here, owing
                    // nothing and carrying no code.
                    match crate::trade_flow_parts::pricing::resolve_spend_fee_inputs(
                        draft.order_type,
                    ) {
                        Ok(fees) if code_charges_this_order(fees.as_ref()) => {
                            return error(
                                -3,
                                "a builder code is configured and charges this order, and polymarket.order.poly1271 cannot declare a fee; post it through `post_builder_code` instead",
                            );
                        }
                        Ok(_) => (None, None),
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
                    // Not a control: `refresh_trade_post_inputs` has already
                    // rewritten the draft's fee from these same live settings,
                    // so in the ordinary case this compares two values of one
                    // function microseconds apart. It is kept because the two
                    // are computed by different code — `fee_bound` here,
                    // `quoted_fee_fields` for the draft — and a divergence
                    // between them would otherwise size, cap and display one
                    // fee while declaring another. What actually catches a
                    // repointed code is the ceremony naming it.
                    if let Err(resp) = fee_matches_draft(
                        &draft,
                        &bound.code_hex,
                        bound.builder_bps,
                        bound.builder_fee_micro,
                    ) {
                        return resp;
                    }
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
    // The signature is produced over `prepared.signing_hash`, while the body
    // posted to the venue is built from the order reconstructed out of the
    // preimage. Nothing compared the two: on a resume, a preimage edited in
    // the store would be posted under a signature committing to different
    // bytes, and the mismatch would surface as a venue rejection rather than
    // as a refusal here, with the ceremony already spent. Equal by
    // construction on the fresh path, which is why it costs nothing to check
    // on both.
    let digest = poly1271_digest(&order, chain_id, draft.neg_risk);
    match prepared.hash() {
        Ok(stored) if stored == digest => {}
        Ok(_) => {
            return error(
                -4,
                "prepared order does not hash to its own signing hash; delete the prepared signing artifact and post again",
            );
        }
        Err(resp) => return resp,
    }
    // The digest check proves the artifact agrees with itself, which is all it
    // can prove: a preimage and its hash are one object, and both are public
    // keccak math over values the writer already holds, so anything that can
    // edit one can edit the other. What was never pinned is the ORDER. The
    // review gate compares the review intent to the draft, and
    // `review_intent_hash` pins the review intent, but no comparison reached
    // the order, so a resumed post could name a different token, size, price
    // or maker than the draft the owner reviewed.
    if let Err(resp) = order_matches_draft(&order, &draft, token_id, funder) {
        return resp;
    }
    let salt = match u64::try_from(order.salt) {
        Ok(salt) => salt,
        Err(_) => return error(-4, "order salt does not fit in u64"),
    };
    draft.salt = Some(salt);
    draft.status = "signing_prepared".into();
    draft.last_error = None;
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
            let (declared_fee_micro, advisory) = crate::builder_fee::ceremony_for_order(
                bound,
                draft.order_type,
                &crate::builder_fee::OrderSubject {
                    slug: &draft.slug,
                    question: &draft.question,
                    outcome: &draft.outcome,
                    side: draft.side,
                    size_micro: draft.size_micro,
                    limit_price_micro: draft.limit_price_micro,
                    proceeds_micro: draft.taker_micro,
                },
            );
            match crate::approval::sign_prepared_with_fee(
                ctx,
                wallet,
                &prepared,
                &approval_key,
                declared_fee_micro,
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
                // The daily cap sums `amount_microusd + fee_microusd`, and a
                // market buy's `amount_micro` already contains its fee
                // because the sizing folded it in. Recording the fee again
                // here charged the cap up to 1% more than ever left the
                // wallet. Same rule as the per-order cap.
                fee_microusd: crate::trade_flow_parts::pricing::fee_on_top_of_amount_micro(&draft),
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
                // The daily cap sums `amount_microusd + fee_microusd`, and a
                // market buy's `amount_micro` already contains its fee
                // because the sizing folded it in. Recording the fee again
                // here charged the cap up to 1% more than ever left the
                // wallet. Same rule as the per-order cap.
                fee_microusd: crate::trade_flow_parts::pricing::fee_on_top_of_amount_micro(&draft),
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
                    // Same rule as the other receipt writers: a market buy's
                    // fee is already inside `amount_micro`.
                    fee_microusd: crate::trade_flow_parts::pricing::fee_on_top_of_amount_micro(
                        &draft,
                    ),
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
                // The daily cap sums `amount_microusd + fee_microusd`, and a
                // market buy's `amount_micro` already contains its fee
                // because the sizing folded it in. Recording the fee again
                // here charged the cap up to 1% more than ever left the
                // wallet. Same rule as the per-order cap.
                fee_microusd: crate::trade_flow_parts::pricing::fee_on_top_of_amount_micro(&draft),
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

/// Both posting routes share one prepared-signing key, so a draft prepared
/// under one class must not be signed under the other: that would declare a
/// fee the class refuses, or omit one it requires.
pub fn class_matches_prepared(
    prepared_intent: &str,
    class: PostClass,
) -> Result<(), DispatchResponse> {
    if prepared_intent == class.operation_class() {
        return Ok(());
    }
    Err(error(
        -3,
        format!(
            "this draft was prepared to sign under {prepared_intent}; post it through that route, or cancel and re-stage it"
        ),
    ))
}

/// Whether a configured code actually charges this order.
///
/// A ceiling may legitimately be zero on one side. When it is zero on the side
/// this order pays, the order owes nothing, the fee-bearing route refuses to
/// declare a zero fee, and refusing here as well left a maker-only builder
/// unable to place any FAK or FOK order through either route.
fn code_charges_this_order(
    fees: Option<&crate::trade_flow_parts::pricing::SpendFeeAttribution>,
) -> bool {
    matches!(fees, Some(fees) if fees.inputs.builder_bps > 0)
}

/// Refuses a buy on the fee-bearing class, whose total debit nothing bounds.
///
/// That class exists to declare a maximum fee, and on a buy there is no
/// maximum to declare. The signed CLOB V2 order carries no fee field — V2
/// removed `feeRateBps` — so the signature commits to `makerAmount` and
/// `takerAmount` and says nothing about a fee, while Polymarket's operator
/// supplies the fee when it fills the order. Neither remaining bound helps:
/// the exchange's on-chain maximum-fee check is disabled on both deployments,
/// and onboarding grants the exchange `U256::MAX` pUSD (`wallet.rs`), so what
/// it may pull is capped by the wallet balance and nothing else. A review
/// reproduced a signed 50 pUSD buy settling for 550.
///
/// Declaring a finite fee here would hand Broker a value limit the same
/// signature permits exceeding, and tell the owner a total that is not one.
/// Refusing is the honest answer.
///
/// A sell is unaffected: what leaves the wallet is the shares in
/// `makerAmount`, which the signature does bound, and a fee can only reduce
/// the proceeds it returns.
///
/// Lifting this needs a bound something actually enforces — the exchange's
/// maximum-fee check enabled, or an allowance set to the order's intended
/// maximum instead of `U256::MAX`.
fn fee_bearing_class_can_bound(
    class: PostClass,
    side: crate::polymarket::Side,
) -> Result<(), DispatchResponse> {
    if matches!(class, PostClass::BuilderCodeOrder) && side == crate::polymarket::Side::Buy {
        return Err(error(
            -3,
            "a buy cannot declare a bounded fee on this class: the signed order carries no fee field, the exchange's maximum-fee check is disabled, and its pUSD allowance is unlimited, so nothing caps what the operator may charge on top of the order amount. Posting it would declare a maximum this signature permits exceeding. Sells are unaffected",
        ));
    }
    Ok(())
}

/// Refuses a fee-free order that carries a builder code.
///
/// Extracted so it can be tested at all: the call site sits in the
/// host-dependent post path, and the one-line version there passed the whole
/// suite when deleted.
fn plain_order_carries_no_code(order: &Order) -> Result<(), DispatchResponse> {
    if order.builder != B256::ZERO {
        return Err(error(
            -4,
            "a fee-free order cannot carry a builder code; its fee could not be declared or disclosed, so this will not be signed",
        ));
    }
    Ok(())
}

/// Refuses when an order is not the order its draft describes.
///
/// Every field here is free on the resume path, where the order is rebuilt
/// entirely from the stored artifact. On the fresh path they hold by
/// construction, which is why checking both paths costs nothing. `salt` and
/// `timestamp` are deliberately not compared: both are generated per attempt
/// and neither moves value.
fn order_matches_draft(
    order: &Order,
    draft: &StoreTradeDraft,
    token_id: U256,
    funder: Address,
) -> Result<(), DispatchResponse> {
    let mismatch = |field: &str| {
        Err(error(
            -4,
            format!(
                "prepared order's {field} is not this draft's; it would sign an order the review never covered"
            ),
        ))
    };
    if order.tokenId != token_id {
        return mismatch("token");
    }
    if order.makerAmount != U256::from(draft.maker_micro) {
        return mismatch("maker amount");
    }
    if order.takerAmount != U256::from(draft.taker_micro) {
        return mismatch("taker amount");
    }
    if order.side != draft.side as u8 {
        return mismatch("side");
    }
    // Both are the funder in a well-formed order. An artifact naming another
    // maker signs over a wallet the owner's review never covered, even if the
    // venue would go on to reject the signature.
    if order.maker != funder || order.signer != funder {
        return mismatch("maker wallet");
    }
    if order.signatureType != SIG_TYPE_POLY_1271 {
        return mismatch("signature type");
    }
    Ok(())
}

/// Refuses when a builder-coded order's fee or beneficiary has moved away from
/// the figures its draft was quoted, sized and reviewed against.
///
/// Both post paths anchor here, for the same reason from opposite directions.
/// The resume path reads its figures back out of the prepared artifact, where
/// this catches a tampered one. The fresh path resolves them from live
/// settings, where without this it would size, display and cap against the
/// draft while signing whatever the settings happen to say now: a ceiling
/// raised or a code repointed after the agent revalidated would reach the
/// ceremony having never appeared in the review the agent showed.
///
/// A draft predating this field fails here rather than posting unanchored,
/// which is the same "write revalidate again" the operator already handles.
fn fee_matches_draft(
    draft: &StoreTradeDraft,
    code_hex: &str,
    builder_bps: u16,
    builder_fee_micro: u64,
) -> Result<(), DispatchResponse> {
    if draft.builder_code.as_deref() == Some(code_hex)
        && draft.builder_fee_rate_bps == Some(builder_bps)
        && draft.builder_fee_micro == Some(builder_fee_micro)
    {
        return Ok(());
    }
    Err(error(
        -4,
        "builder fee or beneficiary disagrees with this draft's resolved values; write revalidate again before posting",
    ))
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
    draft: &StoreTradeDraft,
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
    // other proves nothing if the notional is unrelated to the signed order.
    //
    // The floor is `makerAmount` for both sides. A buy's notional is what it
    // gives, which is `makerAmount`. A sell's is its shares at par, and
    // `sell_from_shares` sets `maker_micro = size_micro`, so par IS
    // `makerAmount` too. An earlier revision bound a sell to `takerAmount`,
    // its USD leg, which is strictly below par at any price under $1 and so
    // accepted a fee priced on a fraction of what par demands.
    let maker_amount = micro("maker_amount")?;
    if notional_micro < maker_amount {
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
    // Self-consistency does not anchor the RATE, and the rate is the whole
    // multiplier: shrinking the rate and the ceiling together satisfies both
    // the consistency check and the floor, so a tampered artifact could have
    // declared a hundredth of the real fee and raised a fresh ceremony
    // showing that figure to the owner. Nor does it anchor the beneficiary,
    // which the floor and the consistency check never touch at all.
    //
    // The stored values are still what gets signed, so a lost response
    // resumes the figures the ceremony showed.
    fee_matches_draft(draft, &code_hex, builder_bps, builder_fee_micro)?;
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

    /// A draft whose resolved fee matches the artifact, which is the honest
    /// case. The rate anchor compares the two, so a test that wants the
    /// artifact accepted must agree with it.
    fn draft_for(rate: u16, ceiling_micro: u64) -> StoreTradeDraft {
        let mut draft: StoreTradeDraft = serde_json::from_value(serde_json::json!({
            "id": "0001", "wallet": "alice", "slug": "s", "question": "q",
            "condition_id": "0xabc", "outcome": "YES", "token_id": "111",
            "side": "BUY", "order_type": "FAK", "amount_micro": 10_000_000u64,
            "price_bound_micro": 100_000u64, "limit_price": null,
            "marketable": true, "limit_price_micro": 500_000u64,
            "size_micro": 20_000_000u64, "maker_micro": 10_000_000u64,
            "taker_micro": 20_000_000u64, "tick_micro": 10_000u64,
            "min_order_size_micro": 1_000_000u64, "neg_risk": false,
            "active": true, "closed": false, "order_book_enabled": true,
            "binary_outcomes": true, "best_ask_micro": null,
            "best_bid_micro": null, "book_snapshot_secs": 1u64,
            "status": "signed"
        }))
        .expect("draft fixture");
        draft.builder_fee_rate_bps = Some(rate);
        draft.builder_fee_micro = Some(ceiling_micro);
        draft.builder_code = Some("0xabc".to_owned());
        draft
    }

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

    /// Nothing bounds a buy's total debit: the signed order has no fee field,
    /// the exchange's maximum-fee check is disabled and its pUSD allowance is
    /// unlimited. A review reproduced a signed 50 pUSD buy settling for 550,
    /// so the fee-bearing class must not claim a maximum on a buy.
    #[test]
    fn the_fee_bearing_class_refuses_a_buy_and_allows_a_sell() {
        use crate::polymarket::Side;
        assert!(
            fee_bearing_class_can_bound(PostClass::BuilderCodeOrder, Side::Buy).is_err(),
            "a buy cannot declare a bound the signature does not enforce"
        );
        // A sell's outflow is the shares in makerAmount, which IS signed.
        assert!(
            fee_bearing_class_can_bound(PostClass::BuilderCodeOrder, Side::Sell).is_ok(),
            "a sell's outflow is bounded by the signed order"
        );
        // The fee-free class declares no fee either way, so it is unaffected.
        for side in [Side::Buy, Side::Sell] {
            assert!(fee_bearing_class_can_bound(PostClass::PlainOrder, side).is_ok());
        }
    }

    /// The fee checks all reason about amounts, so none of them can see the
    /// recipient change. An artifact repointed to another builder keeps every
    /// figure the draft agreed to and pays somebody else.
    /// A ceiling zero on the paying side charges nothing, so the fee-free
    /// route is where such an order belongs. Refusing it there as well, which
    /// is what the code did, left a maker-only builder with no route at all
    /// for a FAK or FOK order.
    #[test]
    fn a_code_that_charges_nothing_does_not_belong_on_the_fee_bearing_route() {
        use crate::trade_flow_parts::pricing::{SpendFeeAttribution, SpendFeeInputs};
        let attribution = |builder_bps| SpendFeeAttribution {
            inputs: SpendFeeInputs { builder_bps },
            code_hex: "0xabc".to_owned(),
        };
        assert!(!code_charges_this_order(None), "no code charges nothing");
        assert!(
            !code_charges_this_order(Some(&attribution(0))),
            "a zero rate on the paying side charges nothing"
        );
        assert!(
            code_charges_this_order(Some(&attribution(1))),
            "a nonzero rate must take the fee-bearing route"
        );
    }

    const FUNDER: &str = "0x0000000000000000000000000000000000000002";

    fn order_for(draft: &StoreTradeDraft, builder_code: Option<B256>) -> Order {
        build_order(
            &OrderParams {
                token_id: draft.token_id.parse().expect("token id"),
                maker: FUNDER.parse().expect("funder"),
                quote: LimitQuote {
                    side: draft.side,
                    price_micro: draft.limit_price_micro,
                    size_micro: draft.size_micro,
                    maker_micro: draft.maker_micro,
                    taker_micro: draft.taker_micro,
                },
                builder_code,
                signature_type: SIG_TYPE_POLY_1271,
            },
            1_700_000_000_000,
        )
    }

    /// The fee-free class declares no fee and its ceremony names no
    /// beneficiary, so an order carrying a code under it is charged by the
    /// venue and paid to whoever owns that code, with the owner shown
    /// nothing. The fresh path cannot build one; a stored artifact can, and
    /// the digest check cannot catch it because an artifact and its hash are
    /// one object.
    #[test]
    fn a_fee_free_order_may_not_carry_a_builder_code() {
        let draft = draft_for(100, 100_000);
        assert!(plain_order_carries_no_code(&order_for(&draft, None)).is_ok());
        assert!(
            plain_order_carries_no_code(&order_for(&draft, Some(B256::repeat_byte(0xab)))).is_err(),
            "a fee-free order carrying a code must not be signed"
        );
    }

    /// On a resume every field of the order comes out of the stored artifact,
    /// and the digest check only proves the artifact agrees with itself.
    /// Nothing pinned the order to the draft the owner reviewed.
    #[test]
    fn a_resumed_order_must_be_the_order_its_draft_describes() {
        let draft = draft_for(100, 100_000);
        let token_id: U256 = draft.token_id.parse().expect("token id");
        let funder: Address = FUNDER.parse().expect("funder");
        let honest = order_for(&draft, None);
        order_matches_draft(&honest, &draft, token_id, funder).expect("the honest order matches");

        // Each of these is free on the resume path, and each changes what the
        // owner pays or what they get, while `salt` and `timestamp` are
        // deliberately not compared.
        type Swap = (&'static str, Box<dyn Fn(&mut Order)>);
        let swaps: Vec<Swap> = vec![
            (
                "token",
                Box::new(|o: &mut Order| o.tokenId = U256::from(999u64)),
            ),
            (
                "maker amount",
                Box::new(|o: &mut Order| o.makerAmount = U256::from(900_000_000u64)),
            ),
            (
                "taker amount",
                Box::new(|o: &mut Order| o.takerAmount = U256::from(1u64)),
            ),
            ("side", Box::new(|o: &mut Order| o.side ^= 1)),
            (
                "maker wallet",
                Box::new(|o: &mut Order| o.maker = Address::repeat_byte(0x99)),
            ),
            (
                "signer",
                Box::new(|o: &mut Order| o.signer = Address::repeat_byte(0x99)),
            ),
            (
                "signature type",
                Box::new(|o: &mut Order| o.signatureType = 1),
            ),
        ];
        for (name, swap) in swaps {
            let mut tampered = honest.clone();
            swap(&mut tampered);
            assert!(
                order_matches_draft(&tampered, &draft, token_id, funder).is_err(),
                "a swapped {name} must not be signed"
            );
        }

        // Per-attempt values must NOT refuse, or no resume could ever post.
        for mut regenerated in [honest.clone(), honest.clone()] {
            regenerated.salt = U256::from(12_345u64);
            regenerated.timestamp = U256::from(1_800_000_000_000u64);
            order_matches_draft(&regenerated, &draft, token_id, funder)
                .expect("salt and timestamp are regenerated per attempt");
        }
    }

    #[test]
    fn a_prepared_builder_order_repointed_to_another_code_is_refused() {
        let mut repointed = preimage(100, "10000000", "100000");
        repointed["builder"] = serde_json::json!("0xdef");
        assert!(
            builder_fee_from_prepared(&repointed, OrderType::FAK, &draft_for(100, 100_000))
                .is_err(),
            "a code the draft never named must not post"
        );
        // The same artifact against a draft that does name it is fine, so the
        // refusal is the beneficiary and not some other field.
        let mut agreed = draft_for(100, 100_000);
        agreed.builder_code = Some("0xdef".to_owned());
        assert!(builder_fee_from_prepared(&repointed, OrderType::FAK, &agreed).is_ok());
    }

    /// The posted body is rebuilt from the preimage while the signature
    /// commits to the stored hash, so the post cross-checks one against the
    /// other. This pins the property that makes that check work: an edited
    /// preimage reconstructs to an order with a different digest.
    #[test]
    fn an_edited_preimage_no_longer_hashes_to_the_stored_digest() {
        let honest = serde_json::json!({
            "funder": "0x0000000000000000000000000000000000000002",
            "salt": "7",
            "token_id": "111",
            "maker_amount": "10000000",
            "taker_amount": "20000000",
            "side": 0,
            "signature_type": 3,
            "timestamp_ms": "1700000000000",
            "builder": "0x00000000000000000000000000000000000000000000000000000000000000ab",
        });
        let order = order_from_prepared(&honest).expect("honest preimage");
        let stored = poly1271_digest(&order, crate::polymarket::POLYGON, false);

        for field in ["maker_amount", "taker_amount", "salt", "builder"] {
            let mut tampered = honest.clone();
            tampered[field] = match field {
                "builder" => serde_json::json!(
                    "0x00000000000000000000000000000000000000000000000000000000000000cd"
                ),
                _ => serde_json::json!("1"),
            };
            let rebuilt = order_from_prepared(&tampered).expect("tampered preimage still parses");
            assert_ne!(
                poly1271_digest(&rebuilt, crate::polymarket::POLYGON, false),
                stored,
                "editing {field} must move the digest away from the signed hash"
            );
        }
    }

    #[test]
    fn a_retry_reads_the_stored_fee_rather_than_recomputing_it() {
        let bound = builder_fee_from_prepared(
            &preimage(100, "10000000", "100000"),
            OrderType::FAK,
            &draft_for(100, 100_000),
        )
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
                builder_fee_from_prepared(&value, OrderType::FAK, &draft_for(100, 100_000))
                    .is_err(),
                "{key} must be required"
            );
        }
    }

    #[test]
    fn a_tampered_fee_that_disagrees_with_its_notional_is_refused() {
        // 10 pUSD at 100 bps is 0.1 pUSD; claiming less is caught.
        assert!(
            builder_fee_from_prepared(
                &preimage(100, "10000000", "1"),
                OrderType::FAK,
                &draft_for(100, 100_000)
            )
            .is_err()
        );
    }

    /// A zero rate and a notional unrelated to the signed amount both passed
    /// the old self-consistency check, which only proved the three stored fee
    /// fields agreed with each other.
    /// Self-consistency does not anchor the rate, and the rate is the whole
    /// multiplier. Shrinking the rate and the ceiling together satisfies both
    /// the consistency check and the notional floor, so a tampered artifact
    /// could raise a fresh ceremony showing a hundredth of the real fee.
    #[test]
    fn a_proportionally_shrunk_rate_and_ceiling_is_refused() {
        let live = draft_for(100, 100_000);

        // Internally consistent at 1 bp: 10 pUSD * 1 / 10_000 = 0.001 pUSD,
        // and the notional still clears `maker_amount`.
        let tampered = preimage(1, "10000000", "1000");
        assert!(
            builder_fee_from_prepared(&tampered, OrderType::FAK, &live).is_err(),
            "a 1 bp artifact must not be accepted for a 100 bp configuration"
        );

        // The honest artifact for that same draft is accepted, and the fee it
        // yields is the one the ceremony showed rather than a recomputation.
        let honest = preimage(100, "10000000", "100000");
        let bound = builder_fee_from_prepared(&honest, OrderType::FAK, &live)
            .expect("the artifact matching the draft is accepted");
        assert_eq!(bound.builder_fee_micro, 100_000);

        // A ceiling raised above the draft's is refused too, not just lowered.
        let inflated = preimage(100, "20000000", "200000");
        assert!(builder_fee_from_prepared(&inflated, OrderType::FAK, &live).is_err());

        // The rate must be anchored in its own right, not just via the
        // ceiling: an inflated notional reaches the draft's exact ceiling at
        // the wrong rate — 1e9 * 1 / 10_000 == 100_000 — so the declared
        // amount is right while the advisory would read "0.01% of 1000 pUSD".
        let misleading = preimage(1, "1000000000", "100000");
        assert!(
            builder_fee_from_prepared(&misleading, OrderType::FAK, &live).is_err(),
            "the rate the owner is shown must be anchored too"
        );
    }

    #[test]
    fn a_prepared_builder_order_cannot_declare_a_zero_rate_or_an_unrelated_notional() {
        let mut zero = preimage(100, "10000000", "100000");
        zero["builder_fee_rate_bps"] = serde_json::json!(0);
        zero["builder_fee_ceiling_micro"] = serde_json::json!("0");
        assert!(
            builder_fee_from_prepared(&zero, OrderType::FAK, &draft_for(100, 100_000)).is_err(),
            "a zero rate declares nothing while the order still pays"
        );

        // Internally consistent, but priced on a tenth of what the order gives.
        let mut understated = preimage(100, "1000000", "10000");
        understated["maker_amount"] = serde_json::json!("10000000");
        assert!(
            builder_fee_from_prepared(&understated, OrderType::FAK, &draft_for(100, 100_000))
                .is_err(),
            "the notional must be bound to what the signed order gives"
        );

        // A sell was previously unbound entirely: the side conjunct made the
        // binding buy-only, so any notional passed.
        // A sell binds to par, which is its `makerAmount`. Binding to the USD
        // leg instead accepted a tenth of par at a $0.10 limit.
        let mut sell = preimage(100, "1000000", "10000");
        sell["side"] = serde_json::json!(1);
        sell["maker_amount"] = serde_json::json!("10000000");
        sell["taker_amount"] = serde_json::json!("1000000");
        assert!(
            builder_fee_from_prepared(&sell, OrderType::FAK, &draft_for(100, 100_000)).is_err(),
            "a sell's notional must be bound to par, not to its USD leg"
        );

        // The same sell priced on par is accepted.
        let mut honest_sell = preimage(100, "10000000", "100000");
        honest_sell["side"] = serde_json::json!(1);
        honest_sell["taker_amount"] = serde_json::json!("1000000");
        assert!(
            builder_fee_from_prepared(&honest_sell, OrderType::FAK, &draft_for(100, 100_000))
                .is_ok()
        );

        // A missing floor is refused rather than skipped.
        let mut floorless = preimage(100, "10000000", "100000");
        floorless.as_object_mut().unwrap().remove("maker_amount");
        assert!(
            builder_fee_from_prepared(&floorless, OrderType::FAK, &draft_for(100, 100_000))
                .is_err()
        );

        // A fee-bearing artifact with no builder code cannot be resumed.
        let mut codeless = preimage(100, "10000000", "100000");
        codeless["builder"] = serde_json::json!("");
        assert!(
            builder_fee_from_prepared(&codeless, OrderType::FAK, &draft_for(100, 100_000)).is_err()
        );

        // The honest version of the same order is accepted.
        let honest = preimage(100, "10000000", "100000");
        assert!(
            builder_fee_from_prepared(&honest, OrderType::FAK, &draft_for(100, 100_000)).is_ok()
        );
    }

    /// Both routes share one prepared-signing key, so the guard compares the
    /// stored intent against the class being posted. Asserting only that the
    /// two class strings differ left the guard itself untested.
    /// The guard the comment calls load-bearing, exercised rather than
    /// asserted about: an earlier test compared two class constants, so
    /// deleting the guard changed nothing the suite could see.
    #[test]
    fn the_class_guard_refuses_a_draft_prepared_under_the_other_class() {
        for (stored, posting) in [
            (PostClass::PlainOrder, PostClass::BuilderCodeOrder),
            (PostClass::BuilderCodeOrder, PostClass::PlainOrder),
        ] {
            assert!(
                class_matches_prepared(stored.operation_class(), posting).is_err(),
                "a draft stored as {} must not post as {}",
                stored.operation_class(),
                posting.operation_class()
            );
        }
        for class in [PostClass::PlainOrder, PostClass::BuilderCodeOrder] {
            assert!(class_matches_prepared(class.operation_class(), class).is_ok());
        }
        // An unrelated intent is refused too, not just the sibling class.
        assert!(class_matches_prepared("polymarket.onboard", PostClass::PlainOrder).is_err());
    }

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
