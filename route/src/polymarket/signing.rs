//! Keyless action builders and signature encoders for sealed approvals.

use alloy::primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};

use crate::polymarket::eip712::{self, Batch, CLOB_AUTH_MESSAGE, Call};
use crate::polymarket::order::{self, Order, OrderType};

pub const ACTION_ID_DOMAIN_V1: &str = "bloom.polymarket.action_id.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1HeaderView {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClobAuthAction {
    pub address: Address,
    pub chain_id: u64,
    pub timestamp: u64,
    pub nonce: u32,
    pub message: String,
    pub headers_no_signature: Vec<L1HeaderView>,
    pub signing_hash: B256,
}

pub fn clob_auth_action_and_hash(
    address: Address,
    timestamp: u64,
    nonce: u32,
    chain_id: u64,
) -> ClobAuthAction {
    let signing_hash = eip712::clob_auth_signing_hash(address, timestamp, nonce, chain_id);
    ClobAuthAction {
        address,
        chain_id,
        timestamp,
        nonce,
        message: CLOB_AUTH_MESSAGE.to_string(),
        headers_no_signature: vec![
            L1HeaderView {
                name: crate::polymarket::signer::POLY_ADDRESS.to_string(),
                value: format!("{address:#x}"),
            },
            L1HeaderView {
                name: crate::polymarket::signer::POLY_NONCE.to_string(),
                value: nonce.to_string(),
            },
            L1HeaderView {
                name: crate::polymarket::signer::POLY_TIMESTAMP.to_string(),
                value: timestamp.to_string(),
            },
        ],
        signing_hash,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderAction {
    pub order_view: serde_json::Value,
    pub chain_id: u64,
    pub neg_risk: bool,
    pub signature_type: u8,
    pub signing_hash: B256,
}

pub fn order_action_and_hash(
    order: &Order,
    chain_id: u64,
    neg_risk: bool,
    order_type: OrderType,
) -> OrderAction {
    OrderAction {
        order_view: serde_json::json!({
            "schema": "bloom.polymarket_order_view.v1",
            "salt": order.salt.to_string(),
            "maker": format!("{:#x}", order.maker),
            "signer": format!("{:#x}", order.signer),
            "tokenId": order.tokenId.to_string(),
            "makerAmount": order.makerAmount.to_string(),
            "takerAmount": order.takerAmount.to_string(),
            "side": order.side.to_string(),
            "signatureType": order.signatureType.to_string(),
            "timestamp": order.timestamp.to_string(),
            "metadata": format!("{:#x}", order.metadata),
            "builder": format!("{:#x}", order.builder),
            "orderType": order_type.as_str(),
            "expiration": "0",
            "negRisk": neg_risk,
            "chainId": chain_id,
        }),
        chain_id,
        neg_risk,
        signature_type: order.signatureType,
        signing_hash: order::poly1271_digest(order, chain_id, neg_risk),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallView {
    pub target: Address,
    pub value: U256,
    pub data_prefix_hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletBatchAction {
    pub deposit_wallet: Address,
    pub chain_id: u64,
    pub nonce: u64,
    pub deadline: u64,
    pub calls: Vec<CallView>,
    pub signing_hash: B256,
    pub body_excluding_signature: serde_json::Value,
}

pub fn wallet_batch_action_and_hash(
    deposit_wallet: Address,
    chain_id: u64,
    nonce: u64,
    deadline: u64,
    calls: &[Call],
) -> WalletBatchAction {
    let batch = Batch {
        wallet: deposit_wallet,
        nonce: U256::from(nonce),
        deadline: U256::from(deadline),
        calls: calls.to_vec(),
    };
    let calls_json: Vec<_> = calls
        .iter()
        .map(|call| {
            serde_json::json!({
                "target": format!("{:#x}", call.target),
                "value": call.value.to_string(),
                "data": format!("0x{}", hex::encode(&call.data)),
            })
        })
        .collect();
    WalletBatchAction {
        deposit_wallet,
        chain_id,
        nonce,
        deadline,
        calls: calls
            .iter()
            .map(|call| CallView {
                target: call.target,
                value: call.value,
                data_prefix_hex: format!("0x{}", hex::encode(&call.data[..call.data.len().min(4)])),
            })
            .collect(),
        signing_hash: eip712::batch_signing_hash(&batch, chain_id, deposit_wallet),
        body_excluding_signature: serde_json::json!({
            "type": "WALLET",
            "to": format!("{:#x}", eip712::FACTORY),
            "nonce": nonce.to_string(),
            "depositWalletParams": {
                "depositWallet": format!("{deposit_wallet:#x}"),
                "deadline": deadline.to_string(),
                "calls": calls_json,
            },
        }),
    }
}

pub fn signature_string_from_raw(raw_sig: &[u8]) -> crate::polymarket::Result<String> {
    let signature = alloy::primitives::Signature::from_raw(raw_sig).map_err(|error| {
        crate::polymarket::PolymarketError::signing(format!("decode raw ECDSA signature: {error}"))
    })?;
    Ok(signature.to_string())
}

pub fn poly1271_signature_from_raw(
    order: &Order,
    raw_sig: &[u8],
    chain_id: u64,
    neg_risk: bool,
) -> crate::polymarket::Result<String> {
    order::wrap_poly1271_signature(order, raw_sig, chain_id, neg_risk)
}

pub fn action_id_for(action_kind: &str, signing_hash: &B256) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ACTION_ID_DOMAIN_V1.as_bytes());
    hasher.update(action_kind.as_bytes());
    hasher.update(signing_hash.as_slice());
    format!(
        "pm-{}",
        &hasher.finalize().to_hex()[..crate::polymarket::ACTION_ID_HEX_PREFIX]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_id_is_bound_to_kind_and_hash() {
        assert_ne!(
            action_id_for("polymarket.order.poly1271", &B256::ZERO),
            action_id_for("polymarket.onboarding", &B256::ZERO)
        );
        assert_ne!(
            action_id_for("polymarket.order.poly1271", &B256::ZERO),
            action_id_for("polymarket.order.poly1271", &B256::repeat_byte(1))
        );
    }

    #[test]
    fn signature_encoding_rejects_wrong_length() {
        assert!(signature_string_from_raw(&[0; 64]).is_err());
    }
}
