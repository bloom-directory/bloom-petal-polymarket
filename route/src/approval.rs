use alloy::primitives::{Address, B256, Signature};
use serde::{Deserialize, Serialize};

use crate::prelude::*;
use petal::sdk::{DispatchResponse, HostStatus, SdkError, SignHashOutcome, SignRequest};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedSigning {
    pub operation: String,
    pub intent: String,
    pub owner: String,
    pub signing_hash: String,
    pub preimage: serde_json::Value,
}

impl PreparedSigning {
    pub fn new(
        operation: impl Into<String>,
        intent: impl Into<String>,
        owner: Address,
        signing_hash: B256,
        preimage: serde_json::Value,
    ) -> Self {
        Self {
            operation: operation.into(),
            intent: intent.into(),
            owner: owner.to_checksum(None),
            signing_hash: format!("{signing_hash:#x}"),
            preimage,
        }
    }

    pub fn hash(&self) -> Result<B256, DispatchResponse> {
        self.signing_hash
            .parse()
            .map_err(|err| error(-4, format!("corrupt prepared signing hash: {err}")))
    }

    fn owner(&self) -> Result<Address, DispatchResponse> {
        self.owner
            .parse()
            .map_err(|err| error(-4, format!("corrupt prepared owner: {err}")))
    }

    fn digest(&self) -> Result<String, DispatchResponse> {
        serde_json::to_vec(self)
            .map(|bytes| blake3_hex(&bytes))
            .map_err(|err| error(-4, format!("encode prepared signing: {err}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ApprovalArtifact {
    action_id: String,
    ceremony_url: String,
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
    wallet: &str,
    prepared: &PreparedSigning,
    approval_key: &str,
) -> Result<Vec<u8>, DispatchResponse> {
    let request = SignRequest {
        wallet: wallet.into(),
        hash32: prepared.hash()?.into(),
        purpose: prepared.intent.clone(),
    };
    match petal::sdk::sign_hash(&request) {
        Ok(SignHashOutcome::Signature(signature)) if signature.len() == 65 => {
            validate_existing_approval(approval_key, prepared, None)?;
            let parsed = Signature::from_raw(&signature)
                .map_err(|err| error(-4, format!("invalid host signature: {err}")))?;
            let recovered = parsed
                .recover_address_from_prehash(&prepared.hash()?)
                .map_err(|err| error(-4, format!("signature recovery failed: {err}")))?;
            if recovered != prepared.owner()? {
                return Err(error(-4, "host signature does not match prepared owner"));
            }
            Ok(signature)
        }
        Ok(SignHashOutcome::Signature(signature)) => Err(error(
            -4,
            format!("sign_hash returned {} bytes", signature.len()),
        )),
        Ok(SignHashOutcome::ApprovalRequired {
            action_id,
            ceremony_url,
            expires_ms,
        }) => {
            validate_existing_approval(approval_key, prepared, Some(&action_id))?;
            let artifact = ApprovalArtifact {
                action_id,
                ceremony_url,
                expires_ms,
                prepared_artifact_digest: prepared.digest()?,
                retry_state: "approval_required".into(),
                operation: prepared.operation.clone(),
            };
            match store_put_json(approval_key, &artifact, false) {
                DispatchResponse::Write => Err(error(
                    -2,
                    format!("Sealed Approval required; read {approval_key} and retry this write"),
                )),
                response => Err(response),
            }
        }
        Err(error) => Err(sdk_error(error)),
    }
}

fn validate_existing_approval(
    key: &str,
    prepared: &PreparedSigning,
    returned_action_id: Option<&str>,
) -> Result<(), DispatchResponse> {
    let bytes = match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Ok(()),
        Err(error) => return Err(sdk_error(error)),
    };
    let existing: ApprovalArtifact = serde_json::from_slice(&bytes)
        .map_err(|err| error(-4, format!("corrupt approval artifact: {err}")))?;
    if existing.prepared_artifact_digest != prepared.digest()?
        || existing.operation != prepared.operation
    {
        return Err(error(
            -4,
            "approval artifact does not match prepared operation",
        ));
    }
    if returned_action_id.is_some_and(|action_id| action_id != existing.action_id)
        && existing.expires_ms > now_millis() as u64
    {
        return Err(error(
            -4,
            "host returned a different action id for the same prepared operation",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_digest_binds_preimage() {
        let first = PreparedSigning::new(
            "order",
            "polymarket.order.poly1271",
            Address::ZERO,
            B256::ZERO,
            serde_json::json!({"amount": "1"}),
        );
        let second = PreparedSigning::new(
            "order",
            "polymarket.order.poly1271",
            Address::ZERO,
            B256::ZERO,
            serde_json::json!({"amount": "2"}),
        );
        assert_ne!(first.digest().unwrap(), second.digest().unwrap());
    }
}
