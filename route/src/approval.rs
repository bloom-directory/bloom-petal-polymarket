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
    sign_action_for(ctx, wallet, action, operation_key, None)
}

pub fn operation_identity(key: &str, class: &str, attempt: Option<&str>) -> String {
    match attempt {
        Some(attempt) => format!("{key}:{class}:{attempt}"),
        None => format!("{key}:{class}"),
    }
}

pub fn relayer_attempt(prepared: &PreparedSigning) -> Result<String, DispatchResponse> {
    prepared
        .preimage
        .get("nonce")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n.to_string())
        .ok_or_else(|| error(-4, "prepared relayer batch is missing nonce"))
}

pub fn sign_action_for(
    ctx: &petal::Ctx,
    wallet: &str,
    action: &petal::signing::PreparedAction,
    operation_key: &str,
    attempt: Option<&str>,
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
    let key = operation_identity(operation_key, &action.operation_class, attempt);
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
    let attempt = match prepared.intent.as_str() {
        "polymarket.relayer_batch" | "polymarket.onboard" => Some(relayer_attempt(prepared)?),
        _ => None,
    };
    sign_action_for(
        ctx,
        wallet,
        &action_for_prepared(prepared)?,
        operation_key,
        attempt.as_deref(),
    )
}

/// Read-only projection of Machine's operation state. Legacy artifacts remain
/// visible for recovery with the original package.
pub fn approval_projection(
    ctx: &petal::Ctx,
    wallet: &str,
    key: &str,
    operations: &[String],
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
    for operation in operations {
        match petal::signing::status(ctx, wallet, operation) {
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

/// Locate the current relayer operation from its durable domain record.
pub fn relayer_operation(key: &str, prepared_key: &str) -> Result<Vec<String>, DispatchResponse> {
    match load_prepared_signing(prepared_key)? {
        Some(prepared) => Ok(vec![operation_identity(
            key,
            &prepared.intent,
            Some(&relayer_attempt(&prepared)?),
        )]),
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relayer_nonce_identifies_a_durable_attempt() {
        let prepared = |nonce| {
            PreparedSigning::new(
                "relayer",
                "polymarket.relayer_batch",
                Address::repeat_byte(1),
                vec![1],
                alloy::primitives::keccak256([1]),
                serde_json::json!({"nonce": nonce}),
            )
        };
        let first = prepared(8);
        let reloaded: PreparedSigning =
            serde_json::from_slice(&serde_json::to_vec(&first).unwrap()).unwrap();
        let key = "actions/test/withdraw/approval.json";
        let identity = |p: &PreparedSigning| {
            operation_identity(key, &p.intent, Some(&relayer_attempt(p).unwrap()))
        };
        assert_eq!(identity(&first), identity(&reloaded));
        assert_ne!(identity(&first), identity(&prepared(9)));
        let malformed = PreparedSigning {
            preimage: serde_json::json!({}),
            ..first
        };
        assert!(relayer_attempt(&malformed).is_err());
    }
}
