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
    pub fn from_prepared(prepared: &crate::approval::PreparedSigning) -> Result<Self> {
        let batch = Self::from_preimage(&prepared.preimage)?;
        if batch.typed_data() != prepared.typed_data {
            return Err(PolymarketError::signing(
                "durable batch does not match its typed signing input",
            ));
        }
        Ok(batch)
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
