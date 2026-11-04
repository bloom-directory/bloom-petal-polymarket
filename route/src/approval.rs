use alloy::primitives::{Address, B256, Signature};
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
    pub typed_data: serde_json::Value,
}

impl PreparedSigning {
    pub fn new(
        operation: impl Into<String>,
        intent: impl Into<String>,
        owner: Address,
        signing_preimage: Vec<u8>,
        signing_hash: B256,
        preimage: serde_json::Value,
        typed_data: serde_json::Value,
    ) -> Self {
        Self {
            operation: operation.into(),
            intent: intent.into(),
            owner: owner.to_checksum(None),
            signing_hash: format!("{signing_hash:#x}"),
            signing_preimage_hex: hex::encode(signing_preimage),
            preimage,
            typed_data,
        }
    }

    pub fn owner(&self) -> Result<Address, DispatchResponse> {
        self.owner
            .parse()
            .map_err(|err| error(-4, format!("corrupt prepared owner: {err}")))
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

/// The generic EIP-712 review and payload always come from the same typed data.
pub fn typed_action(
    class: &str,
    typed_data: &serde_json::Value,
) -> Result<petal::signing::PreparedAction, DispatchResponse> {
    let typed: alloy_dyn_abi::TypedData = serde_json::from_value(typed_data.clone())
        .map_err(|e| error(-4, format!("invalid typed data: {e}")))?;
    let domain = typed.domain.separator();
    let message = typed
        .hash_struct()
        .map_err(|e| error(-4, format!("hash typed data: {e}")))?;
    let preimage = [&[0x19, 0x01][..], domain.as_slice(), message.as_slice()].concat();
    let payload = petal::signing::SigningPayload::keccak256(preimage)
        .map_err(|e| error(-4, e.to_string()))?;
    let bytes = serde_json::to_vec(&serde_json::json!({"schema":"bloom.message.review.v1", "kind":"eip712", "typed_data":typed_data}))
        .map_err(|e| error(-4, format!("encode review: {e}")))?;
    let review = petal::signing::ReviewArtifact::new("bloom.message.review", 1, bytes)
        .map_err(|e| error(-4, e.to_string()))?;
    petal::signing::PreparedAction::new(class, payload, review)
        .map_err(|e| error(-4, e.to_string()))
}

pub fn action_for_prepared(
    prepared: &PreparedSigning,
) -> Result<petal::signing::PreparedAction, DispatchResponse> {
    let action = typed_action(&prepared.intent, &prepared.typed_data)?;
    if action.payload.preimage != prepared.signing_preimage()? {
        return Err(error(
            -4,
            "typed review does not match prepared signing payload",
        ));
    }
    Ok(action)
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

/// Validate the returned signature and render v as 27/28 for the venue.
pub fn verified_signature(
    owner: Address,
    hash: B256,
    bytes: &[u8],
) -> Result<Signature, DispatchResponse> {
    let signature =
        Signature::from_raw(bytes).map_err(|err| error(-4, format!("host signature: {err}")))?;
    if signature
        .recover_address_from_prehash(&hash)
        .map_err(|err| error(-4, format!("recover host signature: {err}")))?
        != owner
    {
        return Err(error(-4, "host signature does not match prepared owner"));
    }
    Ok(signature)
}

pub fn sign_prepared(
    ctx: &petal::Ctx,
    wallet: &str,
    prepared: &PreparedSigning,
    operation_key: &str,
) -> Result<Signature, DispatchResponse> {
    let attempt = match prepared.intent.as_str() {
        "polymarket.relayer_batch" | "polymarket.onboard" => Some(relayer_attempt(prepared)?),
        _ => None,
    };
    let signature = sign_action_for(
        ctx,
        wallet,
        &action_for_prepared(prepared)?,
        operation_key,
        attempt.as_deref(),
    )?;
    verified_signature(prepared.owner()?, prepared.hash()?, &signature)
}

/// Read-only projection of Machine's operation state. Legacy artifacts remain
/// visible for recovery with the original package.
pub fn approval_projection(
    ctx: &petal::Ctx,
    wallet: &str,
    _key: &str,
    operations: &[String],
) -> Result<Option<serde_json::Value>, DispatchResponse> {
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
    fn generic_review_rebuilds_the_exact_authentication_payload() {
        let owner = Address::repeat_byte(1);
        let typed_data = crate::polymarket::eip712::clob_auth_typed_data(owner, 100, 0, 137);
        let mut prepared = PreparedSigning::new(
            "login",
            "fixture.login",
            owner,
            crate::polymarket::eip712::clob_auth_signing_preimage(owner, 100, 0, 137),
            crate::polymarket::eip712::clob_auth_signing_hash(owner, 100, 0, 137),
            serde_json::Value::Null,
            typed_data,
        );
        let action = action_for_prepared(&prepared).unwrap();
        assert_eq!(action.review.format, "bloom.message.review");
        assert_eq!(
            action.payload.preimage,
            prepared.signing_preimage().unwrap()
        );
        prepared.typed_data["message"]["timestamp"] = serde_json::json!("101");
        assert!(action_for_prepared(&prepared).is_err());
    }

    #[test]
    fn signatures_are_checked_against_the_prepared_owner_and_normalized() {
        use alloy::signers::{SignerSync, local::PrivateKeySigner};
        let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(7)).unwrap();
        let preimage = vec![1, 2, 3];
        let prepared = PreparedSigning::new(
            "relayer",
            "polymarket.relayer_batch",
            signer.address(),
            preimage.clone(),
            alloy::primitives::keccak256(&preimage),
            serde_json::json!({}),
            serde_json::Value::Null,
        );
        let signature = signer.sign_hash_sync(&prepared.hash().unwrap()).unwrap();
        let mut bytes = signature.as_bytes();
        bytes[64] -= 27;
        assert_eq!(
            verified_signature(prepared.owner().unwrap(), prepared.hash().unwrap(), &bytes)
                .unwrap()
                .to_string(),
            signature.to_string()
        );
        assert!(
            verified_signature(
                prepared.owner().unwrap(),
                prepared.hash().unwrap(),
                &bytes[..64]
            )
            .is_err()
        );
        let wrong_owner = PreparedSigning {
            owner: Address::repeat_byte(9).to_checksum(None),
            ..prepared
        };
        assert!(
            verified_signature(
                wrong_owner.owner().unwrap(),
                wrong_owner.hash().unwrap(),
                &bytes
            )
            .is_err()
        );
    }

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
                serde_json::Value::Null,
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
