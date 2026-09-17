use alloy::primitives::{Address, B256, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::prelude::*;
use petal::sdk::{DispatchResponse, HostStatus, SdkError, SignBatchOutcome};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedSigning {
    pub operation: String,
    pub intent: String,
    pub owner: String,
    pub signing_hash: String,
    pub signing_preimage_hex: String,
    pub preimage: serde_json::Value,
}

impl PreparedSigning {
    pub fn new(
        operation: impl Into<String>,
        intent: impl Into<String>,
        owner: Address,
        signing_preimage: Vec<u8>,
        signing_hash: B256,
        preimage: serde_json::Value,
    ) -> Self {
        Self {
            operation: operation.into(),
            intent: intent.into(),
            owner: owner.to_checksum(None),
            signing_hash: format!("{signing_hash:#x}"),
            signing_preimage_hex: hex::encode(signing_preimage),
            preimage,
        }
    }

    pub fn hash(&self) -> Result<B256, DispatchResponse> {
        self.signing_hash
            .parse()
            .map_err(|err| error(-4, format!("corrupt prepared signing hash: {err}")))
    }

    pub fn signing_preimage(&self) -> Result<Vec<u8>, DispatchResponse> {
        let preimage = hex::decode(&self.signing_preimage_hex)
            .map_err(|err| error(-4, format!("corrupt prepared signing preimage: {err}")))?;
        if preimage.is_empty() || alloy::primitives::keccak256(&preimage) != self.hash()? {
            return Err(error(
                -4,
                "prepared payload does not match its claimed signing hash",
            ));
        }
        Ok(preimage)
    }

    fn owner(&self) -> Result<Address, DispatchResponse> {
        self.owner
            .parse()
            .map_err(|err| error(-4, format!("corrupt prepared owner: {err}")))
    }

    pub(crate) fn digest(&self) -> Result<String, DispatchResponse> {
        serde_json::to_vec(self)
            .map(|bytes| blake3_hex(&bytes))
            .map_err(|err| error(-4, format!("encode prepared signing: {err}")))
    }
}

fn sign_prepared_items(
    ctx: &petal::Ctx,
    wallet: &str,
    prepared: &[&PreparedSigning],
    operation_class: &str,
    approval_key: &str,
    selector: petal::SignSelector,
) -> Result<Vec<Vec<u8>>, DispatchResponse> {
    if prepared.is_empty() {
        return Err(error(-3, "prepared signing batch is empty"));
    }
    let payloads = prepared
        .iter()
        .map(|item| {
            Ok(petal::PayloadSignItem {
                preimage: item.signing_preimage()?,
                claimed_hash: item.hash()?.into(),
            })
        })
        .collect::<Result<Vec<_>, DispatchResponse>>()?;
    let prepared_bytes = serde_json::to_vec(prepared)
        .map_err(|err| error(-4, format!("encode signing batch: {err}")))?;
    let prepared_artifact_digest = approval_binding_digest(prepared, &selector)?;
    let selector_label = selector_label(&selector);
    let approval_hint =
        existing_approval_hint(approval_key, &prepared_artifact_digest, selector_label)?;
    let sent_hint = approval_hint.is_some();
    let claim = batch_claim(ctx, operation_class, &payloads)?;
    match petal::sdk::sign_payload_batch(&petal::PayloadBatchSignRequest {
        wallet: wallet.into(),
        payloads,
        signature_algorithm: "secp256k1-keccak256-recoverable".into(),
        operation_class: operation_class.into(),
        petal_use_claim_jcs: claim,
        claim_assurance_evidence: None,
        approval_hint,
        action: Some(prepared_bytes),
        advisory: None,
        selector: selector.clone(),
        key_ref_jcs: None,
    }) {
        Ok(SignBatchOutcome::Signatures(signatures)) if signatures.len() == prepared.len() => {
            for (signature, item) in signatures.iter().zip(prepared) {
                if signature.len() != 65 {
                    return Err(error(-4, "payload batch returned a non-65-byte signature"));
                }
                let signature = Signature::from_raw(signature)
                    .map_err(|err| error(-4, format!("host signature: {err}")))?;
                if signature
                    .recover_address_from_prehash(&item.hash()?)
                    .map_err(|err| error(-4, format!("recover host signature: {err}")))?
                    != item.owner()?
                {
                    return Err(error(
                        -4,
                        "host batch signature does not match prepared owner",
                    ));
                }
            }
            let _ = petal::sdk::store_del(approval_key);
            Ok(signatures)
        }
        Ok(SignBatchOutcome::Signatures(_)) => Err(error(
            -4,
            "payload batch returned the wrong signature count",
        )),
        Ok(SignBatchOutcome::ApprovalPending {
            action_id,
            expires_ms,
        }) => {
            let artifact = serde_json::json!({
                "action_id": action_id,
                "expires_ms": expires_ms,
                "prepared_artifact_digest": prepared_artifact_digest,
                "retry_state": "approval_required",
                "operation": "signing_batch",
                "selector": selector_label,
                "request_count": prepared.len(),
            });
            match store_put_json(approval_key, &artifact, false) {
                DispatchResponse::Write => Err(error(
                    -2,
                    format!(
                        "Sealed Approval required for action {}; open the owner-visible Bloom status, approve it, then retry the exact write",
                        artifact["action_id"].as_str().unwrap_or_default(),
                    ),
                )),
                response => Err(response),
            }
        }
        Err(SdkError::Host(HostStatus::Denied)) if sent_hint => {
            // The host keeps its own authorization state keyed by the request
            // identity; the stored hint is advisory. A hint the host rejects
            // (for example the policy-eligibility action recorded on first
            // use) would otherwise be resent on every retry, so retire it and
            // let the retry go without one.
            match petal::sdk::store_del(approval_key) {
                Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => {}
                Err(err) => return Err(sdk_error(err)),
            }
            Err(error(
                -2,
                "host rejected the stored approval hint; it was retired, retry the exact write",
            ))
        }
        Err(SdkError::Host(HostStatus::Denied)) => Err(error(
            -2,
            match selector {
                petal::SignSelector::Exact => {
                    "signing denied: the owner may have rejected the ceremony, the wallet policy may not allow this Petal, or the Broker may be unavailable"
                }
                // A single-use approval is consumed when it signs. If the step
                // after signing failed, the retry reaches the host with the
                // same consumed approval until it expires.
                petal::SignSelector::Reusable => {
                    "signing denied: the owner may have rejected the ceremony, the Broker may be unavailable, or an earlier attempt signed but failed afterwards and its single-use approval is spent (wait up to 5 minutes, then retry the exact write)"
                }
            },
        )),
        Err(err) => Err(sdk_error_with_context("sign prepared batch", err)),
    }
}

fn selector_label(selector: &petal::SignSelector) -> &'static str {
    match selector {
        petal::SignSelector::Exact => "exact",
        petal::SignSelector::Reusable => "reusable",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ApprovalArtifact {
    action_id: String,
    expires_ms: u64,
    prepared_artifact_digest: String,
    retry_state: String,
    operation: String,
    /// Absent on artifacts written before the selector was recorded; those
    /// were always exact.
    #[serde(default)]
    selector: Option<String>,
}

impl ApprovalArtifact {
    fn selector_label(&self) -> &str {
        self.selector.as_deref().unwrap_or("exact")
    }
}

/// Which kind of owner approval, if any, a stored approval artifact is waiting
/// on: `Some("exact")`, `Some("reusable")`, `Some("unreadable")` for an
/// artifact that no longer parses, or `None` when nothing is pending.
pub fn pending_approval_selector(key: &str) -> Result<Option<String>, DispatchResponse> {
    let bytes = match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(None),
        Err(error) => return Err(sdk_error(error)),
    };
    Ok(Some(
        serde_json::from_slice::<ApprovalArtifact>(&bytes)
            .map(|existing| existing.selector_label().to_string())
            .unwrap_or_else(|_| "unreadable".to_string()),
    ))
}

pub fn store_prepared_signing(
    key: &str,
    prepared: &PreparedSigning,
) -> Result<String, DispatchResponse> {
    let digest = prepared.digest()?;
    match store_put_json(key, prepared, false) {
        DispatchResponse::Write => Ok(digest),
        response => Err(response),
    }
}

pub fn load_prepared_signing(key: &str) -> Result<Option<PreparedSigning>, DispatchResponse> {
    let bytes = match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(None),
        Err(error) => return Err(sdk_error(error)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|err| error(-4, format!("corrupt prepared signing artifact: {err}")))
}

pub fn store_review_intent(
    key: &str,
    review_intent: &serde_json::Value,
) -> Result<String, DispatchResponse> {
    let bytes = serde_json::to_vec(review_intent)
        .map_err(|err| error(-4, format!("encode review intent: {err}")))?;
    petal::sdk::store_put(key, &bytes, false).map_err(sdk_error)?;
    Ok(blake3_hex(&bytes))
}

pub fn verify_review_intent(key: &str, expected_hash: &str) -> Result<(), DispatchResponse> {
    let bytes = petal::sdk::store_get(key, MAX_STORE_BYTES)
        .map_err(|error| sdk_error_with_context("read review intent", error))?;
    if blake3_hex(&bytes) != expected_hash {
        return Err(error(
            -4,
            "review intent does not match the prepared operation",
        ));
    }
    Ok(())
}

/// Sign one prepared payload under an owner approval bound to its exact bytes.
pub fn sign_prepared(
    ctx: &petal::Ctx,
    wallet: &str,
    prepared: &PreparedSigning,
    approval_key: &str,
) -> Result<Vec<u8>, DispatchResponse> {
    sign_prepared_items(
        ctx,
        wallet,
        &[prepared],
        &prepared.intent,
        approval_key,
        petal::SignSelector::Exact,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| error(-4, "payload batch returned no signature"))
}

/// Sign one prepared payload under a single-use owner approval that is not
/// bound to its bytes. Use this only for payloads carrying a short-lived venue
/// field (such as a server timestamp) that must be rebuilt after the owner
/// approves; the approval stays bound to this Petal, route, wallet, operation
/// class and signature count.
pub fn sign_prepared_reusable(
    ctx: &petal::Ctx,
    wallet: &str,
    prepared: &PreparedSigning,
    approval_key: &str,
) -> Result<Vec<u8>, DispatchResponse> {
    sign_prepared_items(
        ctx,
        wallet,
        &[prepared],
        &prepared.intent,
        approval_key,
        petal::SignSelector::Reusable,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| error(-4, "payload batch returned no signature"))
}

/// Identity a stored approval artifact must match before its action is reused.
/// Exact approvals bind the full prepared payloads. Reusable approvals bind only
/// the fields that stay stable when the payload is rebuilt.
fn approval_binding_digest(
    prepared: &[&PreparedSigning],
    selector: &petal::SignSelector,
) -> Result<String, DispatchResponse> {
    let bytes = match selector {
        petal::SignSelector::Exact => serde_json::to_vec(prepared),
        petal::SignSelector::Reusable => serde_json::to_vec(&serde_json::json!({
            "selector": "reusable",
            "items": prepared
                .iter()
                .map(|item| serde_json::json!({
                    "operation": item.operation,
                    "intent": item.intent,
                    "owner": item.owner,
                }))
                .collect::<Vec<_>>(),
        })),
    }
    .map_err(|err| error(-4, format!("encode approval binding: {err}")))?;
    Ok(blake3_hex(&bytes))
}

/// Return the pending approval action for this exact operation, if the stored
/// artifact still describes it. The artifact is a Petal-local hint: the host
/// independently checks it against its own authorization state, so a stale
/// or mismatched artifact is retired here and a fresh request is made rather
/// than leaving the route stuck on an artifact the owner cannot clear.
fn existing_approval_hint(
    key: &str,
    prepared_artifact_digest: &str,
    selector_label: &str,
) -> Result<Option<String>, DispatchResponse> {
    let bytes = match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(None),
        Err(error) => return Err(sdk_error(error)),
    };
    let matches = serde_json::from_slice::<ApprovalArtifact>(&bytes)
        .ok()
        .filter(|existing| {
            existing.prepared_artifact_digest == prepared_artifact_digest
                && existing.selector_label() == selector_label
        });
    let Some(existing) = matches else {
        match petal::sdk::store_del(key) {
            Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => {}
            Err(error) => return Err(sdk_error(error)),
        }
        return Ok(None);
    };
    Ok((existing.expires_ms > now_millis() as u64).then_some(existing.action_id))
}

fn batch_claim(
    ctx: &petal::Ctx,
    operation_class: &str,
    payloads: &[petal::PayloadSignItem],
) -> Result<Vec<u8>, DispatchResponse> {
    let route = ctx
        .params
        .iter()
        .find_map(|(name, value)| (name == "bloom.route_id").then_some(value.as_str()))
        .ok_or_else(|| error(-4, "trusted Petal route id is unavailable"))?;
    let payload_digest = petal::payload_batch_digest(payloads).map_err(sdk_error)?;
    let ordered_hashes = payloads
        .iter()
        .map(|payload| hex::encode(payload.claimed_hash))
        .collect::<Vec<_>>();
    let nonce = Sha256::digest(
        [
            ctx.package_hash.as_bytes(),
            route.as_bytes(),
            operation_class.as_bytes(),
            payload_digest.as_slice(),
        ]
        .concat(),
    );
    serde_jcs::to_vec(&serde_json::json!({
        "package_hash": ctx.package_hash,
        "route": route,
        "operation_class": operation_class,
        "crypto_suite": "secp256k1-keccak256-recoverable",
        "payload_digest": hex::encode(payload_digest),
        "ordered_hashes": ordered_hashes,
        "declared_debits": [],
        "declared_destinations": [],
        "declared_fee": {"kind": "none"},
        "nonce": hex::encode(&nonce[..16]),
        "claim_assurance": {"kind": "machine_asserted"}
    }))
    .map_err(|err| error(-4, format!("encode Petal use claim: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_digest_binds_preimage() {
        let signing_preimage = vec![1, 2, 3];
        let signing_hash = alloy::primitives::keccak256(&signing_preimage);
        let first = PreparedSigning::new(
            "order",
            "polymarket.order.poly1271",
            Address::ZERO,
            signing_preimage.clone(),
            signing_hash,
            serde_json::json!({"amount": "1"}),
        );
        let second = PreparedSigning::new(
            "order",
            "polymarket.order.poly1271",
            Address::ZERO,
            signing_preimage,
            signing_hash,
            serde_json::json!({"amount": "2"}),
        );
        assert_ne!(first.digest().unwrap(), second.digest().unwrap());
    }

    #[test]
    fn approval_artifact_without_selector_reads_as_exact() {
        let legacy: ApprovalArtifact = serde_json::from_value(serde_json::json!({
            "action_id": "a",
            "expires_ms": 1,
            "prepared_artifact_digest": "d",
            "retry_state": "approval_required",
            "operation": "signing_batch",
        }))
        .unwrap();
        assert_eq!(legacy.selector_label(), "exact");
        let reusable: ApprovalArtifact = serde_json::from_value(serde_json::json!({
            "action_id": "a",
            "expires_ms": 1,
            "prepared_artifact_digest": "d",
            "retry_state": "approval_required",
            "operation": "signing_batch",
            "selector": "reusable",
        }))
        .unwrap();
        assert_eq!(reusable.selector_label(), "reusable");
    }

    fn clob_auth_at(timestamp: u64) -> PreparedSigning {
        let signing_preimage = timestamp.to_be_bytes().to_vec();
        PreparedSigning::new(
            "clob_auth",
            "polymarket.onboard",
            Address::ZERO,
            signing_preimage.clone(),
            alloy::primitives::keccak256(&signing_preimage),
            serde_json::json!({"timestamp": timestamp}),
        )
    }

    #[test]
    fn exact_approval_binding_changes_with_rebuilt_payload() {
        let first = clob_auth_at(1_000);
        let rebuilt = clob_auth_at(1_300);
        assert_ne!(
            approval_binding_digest(&[&first], &petal::SignSelector::Exact).unwrap(),
            approval_binding_digest(&[&rebuilt], &petal::SignSelector::Exact).unwrap()
        );
    }

    #[test]
    fn reusable_approval_binding_survives_rebuilt_payload() {
        let first = clob_auth_at(1_000);
        let rebuilt = clob_auth_at(1_300);
        assert_eq!(
            approval_binding_digest(&[&first], &petal::SignSelector::Reusable).unwrap(),
            approval_binding_digest(&[&rebuilt], &petal::SignSelector::Reusable).unwrap()
        );
        assert_ne!(
            approval_binding_digest(&[&first], &petal::SignSelector::Reusable).unwrap(),
            approval_binding_digest(&[&first], &petal::SignSelector::Exact).unwrap()
        );
    }

    /// The Petal-local hint distinguishes operation and owner so a pending
    /// CLOB-auth artifact is never presented as the hint for another prepared
    /// operation. This is advisory only: the Broker's reusable approval scope
    /// is keyed on the operation class, which both onboarding payloads share.
    #[test]
    fn reusable_approval_hint_distinguishes_operation_and_owner() {
        let clob = clob_auth_at(1_000);
        let mut other_owner = clob.clone();
        other_owner.owner = "0x0000000000000000000000000000000000000001".into();
        let mut other_operation = clob.clone();
        other_operation.operation = "onboard_approvals".into();
        let base = approval_binding_digest(&[&clob], &petal::SignSelector::Reusable).unwrap();
        for changed in [other_owner, other_operation] {
            assert_ne!(
                base,
                approval_binding_digest(&[&changed], &petal::SignSelector::Reusable).unwrap()
            );
        }
    }
}
