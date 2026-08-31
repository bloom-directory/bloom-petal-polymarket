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

pub fn sign_prepared_batch(
    ctx: &petal::Ctx,
    wallet: &str,
    prepared: &[&PreparedSigning],
    operation_class: &str,
    approval_key: &str,
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
    let prepared_artifact_digest = blake3_hex(&prepared_bytes);
    let approval_hint = existing_approval_hint(approval_key, &prepared_artifact_digest)?;
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
        selector: petal::SignSelector::Exact,
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
        Err(err) => Err(sdk_error_with_context("sign prepared batch", err)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ApprovalArtifact {
    action_id: String,
    expires_ms: u64,
    prepared_artifact_digest: String,
    retry_state: String,
    operation: String,
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

pub fn sign_prepared(
    ctx: &petal::Ctx,
    wallet: &str,
    prepared: &PreparedSigning,
    approval_key: &str,
) -> Result<Vec<u8>, DispatchResponse> {
    sign_prepared_batch(ctx, wallet, &[prepared], &prepared.intent, approval_key)?
        .into_iter()
        .next()
        .ok_or_else(|| error(-4, "payload batch returned no signature"))
}

fn existing_approval_hint(
    key: &str,
    prepared_artifact_digest: &str,
) -> Result<Option<String>, DispatchResponse> {
    let bytes = match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(None),
        Err(error) => return Err(sdk_error(error)),
    };
    let existing: ApprovalArtifact = serde_json::from_slice(&bytes)
        .map_err(|err| error(-4, format!("corrupt approval artifact: {err}")))?;
    if existing.prepared_artifact_digest != prepared_artifact_digest {
        return Err(error(
            -4,
            "approval artifact does not match prepared operation",
        ));
    }
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
}
