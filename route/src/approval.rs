use alloy::primitives::{Address, B256};
use serde::{Deserialize, Serialize};

use crate::prelude::*;
use petal::sdk::{DispatchResponse, HostStatus, SdkError};

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

    pub(crate) fn digest(&self) -> Result<String, DispatchResponse> {
        serde_json::to_vec(self)
            .map(|bytes| blake3_hex(&bytes))
            .map_err(|err| error(-4, format!("encode prepared signing: {err}")))
    }
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

/// Build the existing Broker witness once for the shared lifecycle adapter.
pub fn action_for_prepared(
    prepared: &PreparedSigning,
) -> Result<petal::signing::PreparedAction, DispatchResponse> {
    let review =
        serde_json::to_vec(&[prepared]).map_err(|e| error(-4, format!("encode review: {e}")))?;
    let payload = petal::signing::SigningPayload::keccak256(prepared.signing_preimage()?)
        .map_err(|e| error(-4, e.to_string()))?;
    let review = petal::signing::ReviewArtifact::new("polymarket.prepared-record", 1, review)
        .map_err(|e| error(-4, e.to_string()))?;
    petal::signing::PreparedAction::new(&prepared.intent, payload, review)
        .map_err(|e| error(-4, e.to_string()))
}

pub fn sign_action(
    ctx: &petal::Ctx,
    wallet: &str,
    action: &petal::signing::PreparedAction,
    operation_key: &str,
) -> Result<Vec<u8>, DispatchResponse> {
    // Old hints belong to the previous package and must never be reinterpreted
    // as authority by this one. Leave them inspectable for the old build.
    match petal::sdk::store_get(operation_key, MAX_STORE_BYTES) {
        Ok(_) => {
            return Err(error(
                -3,
                "legacy approval pending; reconcile with the original package",
            ));
        }
        Err(SdkError::Host(HostStatus::NotFound)) => (),
        Err(e) => return Err(sdk_error(e)),
    }
    let key = format!("{}:{}", operation_key, action.operation_class);
    match petal::signing::sign(ctx, wallet, action, &key) {
        Ok(petal::signing::SigningOutcome::Signed { signature }) => Ok(signature),
        Ok(petal::signing::SigningOutcome::ApprovalPending { action_id, .. }) => Err(error(
            -3,
            format!(
                "Sealed Approval required for action {action_id}; approve it in Bloom, then retry"
            ),
        )),
        Ok(petal::signing::SigningOutcome::RecoveryRequired { status }) => {
            Err(error(-3, format!("signing recovery required: {status}")))
        }
        Err(e) => Err(error(-3, format!("signing failed: {e}"))),
    }
}

pub fn sign_prepared(
    ctx: &petal::Ctx,
    wallet: &str,
    prepared: &PreparedSigning,
    operation_key: &str,
) -> Result<Vec<u8>, DispatchResponse> {
    sign_action(ctx, wallet, &action_for_prepared(prepared)?, operation_key)
}

/// Read-only projection of Machine's operation state. Legacy artifacts remain
/// visible for recovery with the original package.
pub fn approval_projection(
    ctx: &petal::Ctx,
    wallet: &str,
    key: &str,
    classes: &[&str],
) -> Result<Option<serde_json::Value>, DispatchResponse> {
    match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => {
            return serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| error(-4, format!("corrupt legacy approval: {e}")));
        }
        Err(SdkError::Host(HostStatus::NotFound)) => (),
        Err(e) => return Err(sdk_error(e)),
    }
    for class in classes {
        match petal::signing::status(ctx, wallet, &format!("{key}:{class}")) {
            Ok(petal::signing::SigningOutcome::ApprovalPending {
                action_id,
                expires_ms,
            }) => {
                return Ok(Some(
                    serde_json::json!({"action_id": action_id, "expires_ms": expires_ms, "retry_state": "approval_required"}),
                ));
            }
            Ok(petal::signing::SigningOutcome::RecoveryRequired { status }) => {
                return Ok(Some(
                    serde_json::json!({"retry_state": "recovery_required", "status": status}),
                ));
            }
            Ok(petal::signing::SigningOutcome::Signed { .. }) => (),
            Err(e)
                if e.code == petal::signing::ErrorCode::Invalid
                    && e.message == "operation not found" => {}
            Err(e) => return Err(error(-4, format!("read signing status: {e}"))),
        }
    }
    Ok(None)
}
