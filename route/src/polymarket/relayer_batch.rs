//! Durable relayer batch state; authorization remains with Bloom.
use super::eip712::{Batch, Call};
use super::{PolymarketError, Result};
use alloy::primitives::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

/// One call of a deposit-wallet batch as the route persists it and sends it to
/// the relayer: an EIP-55 `target`, a decimal `value` and 0x-hex `data`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchCall {
    #[serde(with = "alloy::serde::checksum")]
    pub target: Address,
    #[serde(with = "alloy::serde::displayfromstr")]
    pub value: U256,
    pub data: Bytes,
}

impl From<&Call> for BatchCall {
    fn from(call: &Call) -> Self {
        Self {
            target: call.target,
            value: call.value,
            data: call.data.clone(),
        }
    }
}

impl From<&BatchCall> for Call {
    fn from(call: &BatchCall) -> Self {
        Self {
            target: call.target,
            value: call.value,
            data: call.data.clone(),
        }
    }
}

/// Durable domain input bound to the exact typed batch and local review intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedBatch {
    #[serde(with = "alloy::serde::checksum")]
    pub deposit_wallet: Address,
    pub chain_id: u64,
    pub nonce: u64,
    pub deadline: u64,
    pub calls: Vec<BatchCall>,
    pub review_intent_hash: String,
}

impl PreparedBatch {
    pub fn from_preimage(preimage: &serde_json::Value) -> Result<Self> {
        Self::deserialize(preimage)
            .map_err(|err| PolymarketError::signing(format!("prepared batch: {err}")))
    }

    pub fn to_preimage(&self) -> Result<serde_json::Value> {
        serde_json::to_value(self)
            .map_err(|err| PolymarketError::signing(format!("encode prepared batch: {err}")))
    }
}

impl PreparedBatch {
    pub fn signing_hash(&self) -> alloy::primitives::B256 {
        let batch = Batch {
            wallet: self.deposit_wallet,
            nonce: U256::from(self.nonce),
            deadline: U256::from(self.deadline),
            calls: self.calls.iter().map(Call::from).collect(),
        };
        super::eip712::batch_signing_hash(&batch, self.chain_id, self.deposit_wallet)
    }

    pub fn typed_data(&self) -> serde_json::Value {
        let batch = Batch {
            wallet: self.deposit_wallet,
            nonce: U256::from(self.nonce),
            deadline: U256::from(self.deadline),
            calls: self.calls.iter().map(Call::from).collect(),
        };
        super::eip712::batch_typed_data(&batch, self.chain_id, self.deposit_wallet)
    }
}

/// The domain pointer names a batch nonce, not a cached signing outcome.
/// It is cleared only once the venue confirms the previous batch.
pub fn current_scope(
    host: &mut impl petal::signing::SigningHost,
    base: &str,
) -> std::result::Result<Option<String>, petal::DispatchResponse> {
    host.store_get(&format!("{base}/current_batch_nonce.json"))
        .map_err(petal::DispatchResponse::from)?
        .map(|bytes| {
            serde_json::from_slice::<u64>(&bytes)
                .map(|nonce| format!("{base}/batch-{nonce}"))
                .map_err(|e| petal::error(-4, format!("invalid batch nonce: {e}")))
        })
        .transpose()
}

pub fn begin_scope(
    host: &mut impl petal::signing::SigningHost,
    base: &str,
    nonce: u64,
) -> std::result::Result<String, petal::DispatchResponse> {
    if let Some(scope) = current_scope(host, base)? {
        return Ok(scope);
    }
    host.store_put_new(
        &format!("{base}/current_batch_nonce.json"),
        nonce.to_string().as_bytes(),
    )
    .map_err(petal::DispatchResponse::from)?;
    current_scope(host, base)?
        .ok_or_else(|| petal::error(-4, "missing batch nonce after initialization"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_batch_pointer_keeps_its_nonce_across_retry_and_restart() {
        let mut host = crate::onboarding::tests::Host::default();
        let first = begin_scope(&mut host, "actions/test/withdraw", 8).unwrap();
        let mut restarted = crate::onboarding::tests::Host::default();
        restarted.store = host.store.clone();
        assert_eq!(
            begin_scope(&mut restarted, "actions/test/withdraw", 9).unwrap(),
            first
        );
        assert_eq!(
            restarted.store, host.store,
            "retry cannot replace a pending batch pointer"
        );
        restarted.store.insert(
            "actions/test/withdraw/current_batch_nonce.json".into(),
            b"bad".to_vec(),
        );
        assert!(begin_scope(&mut restarted, "actions/test/withdraw", 10).is_err());
    }
    #[test]
    fn batch_context_round_trip_matches_sdk_payload_and_owner_signature() {
        use alloy::signers::{SignerSync, local::PrivateKeySigner};
        let signer =
            PrivateKeySigner::from_bytes(&alloy::primitives::B256::repeat_byte(7)).unwrap();
        let batch = PreparedBatch {
            deposit_wallet: Address::repeat_byte(2),
            chain_id: 137,
            nonce: 8,
            deadline: 500,
            calls: super::super::wallet::approval_calls()
                .iter()
                .map(BatchCall::from)
                .collect(),
            review_intent_hash: "review".into(),
        };
        let restored: PreparedBatch =
            serde_json::from_slice(&serde_json::to_vec(&batch).unwrap()).unwrap();
        let action =
            petal::signing::eip712_action("polymarket.onboard", &restored.typed_data()).unwrap();
        assert_eq!(
            action.payload.hash.as_slice(),
            batch.signing_hash().as_slice()
        );
        let signature = signer.sign_hash_sync(&batch.signing_hash()).unwrap();
        assert_eq!(
            crate::approval::verified_signature(
                signer.address(),
                restored.signing_hash(),
                &signature.as_bytes()
            )
            .unwrap(),
            signature
        );
        let changed = PreparedBatch {
            nonce: 9,
            ..restored
        };
        assert!(
            crate::approval::verified_signature(
                signer.address(),
                changed.signing_hash(),
                &signature.as_bytes()
            )
            .is_err()
        );
    }
}
