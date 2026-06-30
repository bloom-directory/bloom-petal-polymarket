use crate::prelude::*;

use petal::sdk::DispatchResponse;
use crate::polymarket::eip712::{
    CTF, CTF_COLLATERAL_ADAPTER, CTF_EXCHANGE_V2, FACTORY, NEG_RISK_CTF_COLLATERAL_ADAPTER,
    NEG_RISK_EXCHANGE_V2, PUSD,
};
use crate::polymarket::{Credentials, Result};
use alloy::primitives::{Address, U256};
pub(crate) fn read_chain_ctf_balance(
    deposit: Address,
    token_id: &str,
) -> Result<u64, DispatchResponse> {
    let response = read_chain_method(
        CTF,
        "balanceOf",
        &serde_json::json!({
            "args": [deposit.to_checksum(None), token_id]
        }),
    )?;
    let decoded = response
        .get("decoded")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| error(-4, "chain CTF balanceOf response missing decoded array"))?;
    let raw = decoded
        .first()
        .ok_or_else(|| error(-4, "chain CTF balanceOf response missing balance"))?;
    parse_clob_raw_micro(raw).ok_or_else(|| error(-4, "chain CTF balance is not a u64"))
}

pub(crate) fn read_chain_ctf_approval(
    deposit: Address,
    operator: Address,
) -> Result<bool, DispatchResponse> {
    let response = read_chain_method(
        CTF,
        "isApprovedForAll",
        &serde_json::json!({
            "args": [deposit.to_checksum(None), operator.to_checksum(None)]
        }),
    )?;
    let decoded = response
        .get("decoded")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            error(
                -4,
                "chain CTF isApprovedForAll response missing decoded array",
            )
        })?;
    decoded
        .first()
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| error(-4, "chain CTF approval response is not a boolean"))
}

pub(crate) fn read_chain_deposit_wallet_deployed(
    address: Address,
) -> Result<bool, DispatchResponse> {
    let path = format!(
        "chains/polygon/contracts/{}/proxy/implementation",
        address.to_checksum(None)
    );
    let bytes = petal::sdk::vfs_read(&path, MAX_CHAIN_READ_BYTES)
        .map_err(|e| sdk_error_with_context("read deposit wallet proxy implementation", e))?;
    let text = core::str::from_utf8(&bytes)
        .map_err(|_| error(-4, "chain proxy implementation response is not UTF-8"))?;
    let text = text.trim();
    if text == "not a proxy" {
        return Ok(false);
    }
    text.parse::<Address>().map(|_| true).map_err(|e| {
        error(
            -4,
            format!("chain proxy implementation response is not an address: {e}"),
        )
    })
}

pub(crate) fn read_chain_erc20_balance(
    token: Address,
    holder: Address,
) -> Result<U256, DispatchResponse> {
    let response = read_chain_method(
        token,
        "balanceOf",
        &serde_json::json!({
            "args": [holder.to_checksum(None)]
        }),
    )?;
    read_decoded_u256(&response, "chain ERC20 balanceOf")
}

pub(crate) fn read_chain_erc20_allowance(
    token: Address,
    owner: Address,
    spender: Address,
) -> Result<U256, DispatchResponse> {
    let response = read_chain_method(
        token,
        "allowance",
        &serde_json::json!({
            "args": [owner.to_checksum(None), spender.to_checksum(None)]
        }),
    )?;
    read_decoded_u256(&response, "chain ERC20 allowance")
}

pub(crate) fn read_chain_v2_approvals(deposit: Address) -> Result<bool, DispatchResponse> {
    let floor = allowance_floor();
    for spender in v2_spenders() {
        if read_chain_erc20_allowance(PUSD, deposit, spender)? < floor {
            return Ok(false);
        }
    }
    for operator in v2_spenders() {
        if !read_chain_ctf_approval(deposit, operator)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn read_clob_collateral_sync(
    owner: Address,
    creds: &Credentials,
) -> Result<(bool, Option<U256>, Option<U256>), DispatchResponse> {
    let value = clob_l2_get_json(
        owner,
        creds,
        "/balance-allowance",
        &[("asset_type", "COLLATERAL"), ("signature_type", "3")],
    )?;
    let balance = value.get("balance").and_then(parse_json_u256);
    let allowance = value.get("allowance").and_then(parse_json_u256);
    Ok((
        balance.map(|v| !v.is_zero()).unwrap_or(false)
            && allowance.map(|v| !v.is_zero()).unwrap_or(false),
        balance,
        allowance,
    ))
}

pub(crate) fn read_decoded_u256(
    response: &serde_json::Value,
    label: &str,
) -> Result<U256, DispatchResponse> {
    let decoded = response
        .get("decoded")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| error(-4, format!("{label} response missing decoded array")))?;
    let raw = decoded
        .first()
        .ok_or_else(|| error(-4, format!("{label} response missing value")))?;
    parse_json_u256(raw).ok_or_else(|| error(-4, format!("{label} response is not a uint256")))
}

pub(crate) fn parse_json_u256(value: &serde_json::Value) -> Option<U256> {
    match value {
        serde_json::Value::String(s) => s.trim().parse::<U256>().ok(),
        serde_json::Value::Number(n) => n.as_u64().map(U256::from),
        _ => None,
    }
}

pub(crate) fn allowance_floor() -> U256 {
    U256::from(1) << 160
}

pub(crate) fn v2_spenders() -> [Address; 4] {
    [
        CTF_EXCHANGE_V2,
        NEG_RISK_EXCHANGE_V2,
        CTF_COLLATERAL_ADAPTER,
        NEG_RISK_CTF_COLLATERAL_ADAPTER,
    ]
}

pub(crate) fn predict_deposit_wallet(owner: Address) -> Result<Address, DispatchResponse> {
    let implementation = read_chain_address(
        FACTORY,
        "implementation",
        &serde_json::json!({ "args": [] }),
        "factory implementation",
    )?;
    let wallet_id = format!("0x{}{}", "00".repeat(12), hex::encode(owner.as_slice()));
    read_chain_address(
        FACTORY,
        "predictWalletAddress",
        &serde_json::json!({
            "args": [implementation.to_checksum(None), wallet_id]
        }),
        "factory predictWalletAddress",
    )
}

pub(crate) fn read_chain_address(
    contract: Address,
    method: &str,
    body: &serde_json::Value,
    label: &str,
) -> Result<Address, DispatchResponse> {
    let response = read_chain_method(contract, method, body)?;
    let decoded = response
        .get("decoded")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| error(-4, format!("{label} response missing decoded array")))?;
    decoded
        .first()
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, format!("{label} response is not an address")))?
        .parse::<Address>()
        .map_err(|e| error(-4, format!("{label} address parse: {e}")))
}

pub(crate) fn read_chain_method(
    contract: Address,
    method: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, DispatchResponse> {
    let nonce = chain_method_nonce();
    let path = format!(
        "chains/polygon/contracts/{}/methods/{method}@{nonce}.read",
        contract.to_checksum(None)
    );
    let bytes =
        serde_json::to_vec(body).map_err(|e| error(-4, format!("chain method body: {e}")))?;
    petal::sdk::vfs_write(&path, &bytes)
        .map_err(|e| sdk_error_with_context("stage chain method read", e))?;
    let response = petal::sdk::vfs_read(&path, MAX_CHAIN_METHOD_BYTES)
        .map_err(|e| sdk_error_with_context("read chain method result", e))?;
    serde_json::from_slice(&response).map_err(|e| error(-4, format!("chain method JSON: {e}")))
}

pub(crate) fn chain_method_nonce() -> String {
    let bytes = petal::sdk::random_bytes(16).unwrap_or_else(|_| {
        let mut fallback = [0u8; 16];
        fallback[..8].copy_from_slice(&now_millis().to_be_bytes());
        fallback.to_vec()
    });
    hex::encode(bytes)
}
