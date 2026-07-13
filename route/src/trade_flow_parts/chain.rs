use crate::prelude::*;

use crate::polymarket::eip712::{
    CTF, CTF_COLLATERAL_ADAPTER, CTF_EXCHANGE_V2, FACTORY, NEG_RISK_CTF_COLLATERAL_ADAPTER,
    NEG_RISK_EXCHANGE_V2, PUSD,
};
use crate::polymarket::{Credentials, Result};
use alloy::primitives::{Address, U256};
use alloy::sol;
use alloy::sol_types::SolCall;
use petal::sdk::DispatchResponse;

sol! {
    interface DepositWalletFactory {
        function implementation() external view returns (address);
        function predictWalletAddress(address implementation, bytes32 walletId)
            external view returns (address);
    }

    interface Erc20Reads {
        function balanceOf(address holder) external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
    }

    interface CtfReads {
        function balanceOf(address holder, uint256 tokenId) external view returns (uint256);
        function isApprovedForAll(address owner, address operator) external view returns (bool);
    }
}

pub fn read_chain_ctf_balance(deposit: Address, token_id: &str) -> Result<u64, DispatchResponse> {
    let token_id = token_id
        .parse::<U256>()
        .map_err(|err| error(-4, format!("CTF token id is not a uint256: {err}")))?;
    let output = read_chain_eth_call(
        CTF,
        &CtfReads::balanceOfCall {
            holder: deposit,
            tokenId: token_id,
        }
        .abi_encode(),
        "chain CTF balanceOf",
    )?;
    let decoded = CtfReads::balanceOfCall::abi_decode_returns(&output)
        .map_err(|err| error(-4, format!("chain CTF balanceOf decode: {err}")))?;
    u64::try_from(decoded).map_err(|_| error(-4, "chain CTF balance exceeds u64"))
}

pub fn read_chain_ctf_approval(
    deposit: Address,
    operator: Address,
) -> Result<bool, DispatchResponse> {
    let output = read_chain_eth_call(
        CTF,
        &CtfReads::isApprovedForAllCall {
            owner: deposit,
            operator,
        }
        .abi_encode(),
        "chain CTF isApprovedForAll",
    )?;
    CtfReads::isApprovedForAllCall::abi_decode_returns(&output)
        .map_err(|err| error(-4, format!("chain CTF isApprovedForAll decode: {err}")))
}

pub fn read_chain_deposit_wallet_deployed(address: Address) -> Result<bool, DispatchResponse> {
    let (chain, _) = crate::runtime_config::chain().map_err(|err| error(-4, err))?;
    let result = petal::sdk::chain_read(
        &chain,
        "eth_getCode",
        &serde_json::json!([address.to_checksum(None), "latest"]).to_string(),
    )
    .map_err(|err| sdk_error_with_context("read deposit wallet contract code", err))?;
    let result: String = serde_json::from_str(&result)
        .map_err(|err| error(-4, format!("deposit wallet contract code JSON: {err}")))?;
    let encoded = result
        .strip_prefix("0x")
        .ok_or_else(|| error(-4, "deposit wallet contract code is not hex"))?;
    let code = hex::decode(encoded)
        .map_err(|err| error(-4, format!("deposit wallet contract code hex: {err}")))?;
    Ok(!code.is_empty())
}

pub fn read_chain_erc20_balance(token: Address, holder: Address) -> Result<U256, DispatchResponse> {
    let output = read_chain_eth_call(
        token,
        &Erc20Reads::balanceOfCall { holder }.abi_encode(),
        "chain ERC20 balanceOf",
    )?;
    Erc20Reads::balanceOfCall::abi_decode_returns(&output)
        .map_err(|err| error(-4, format!("chain ERC20 balanceOf decode: {err}")))
}

pub fn read_chain_erc20_allowance(
    token: Address,
    owner: Address,
    spender: Address,
) -> Result<U256, DispatchResponse> {
    let output = read_chain_eth_call(
        token,
        &Erc20Reads::allowanceCall { owner, spender }.abi_encode(),
        "chain ERC20 allowance",
    )?;
    Erc20Reads::allowanceCall::abi_decode_returns(&output)
        .map_err(|err| error(-4, format!("chain ERC20 allowance decode: {err}")))
}

pub fn read_chain_v2_approvals(deposit: Address) -> Result<bool, DispatchResponse> {
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

pub fn read_clob_collateral_sync(
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
    let allowance = clob_collateral_allowance(&value);
    Ok((
        balance.map(|v| !v.is_zero()).unwrap_or(false)
            && allowance.map(|v| !v.is_zero()).unwrap_or(false),
        balance,
        allowance,
    ))
}

fn clob_collateral_allowance(value: &serde_json::Value) -> Option<U256> {
    value
        .get("allowance")
        .and_then(parse_json_u256)
        .or_else(|| {
            value
                .get("allowances")
                .and_then(serde_json::Value::as_object)
                .and_then(|allowances| allowances.values().filter_map(parse_json_u256).max())
        })
}

pub fn parse_json_u256(value: &serde_json::Value) -> Option<U256> {
    match value {
        serde_json::Value::String(s) => s.trim().parse::<U256>().ok(),
        serde_json::Value::Number(n) => n.as_u64().map(U256::from),
        _ => None,
    }
}

pub fn allowance_floor() -> U256 {
    U256::from(1) << 160
}

pub fn v2_spenders() -> [Address; 4] {
    [
        CTF_EXCHANGE_V2,
        NEG_RISK_EXCHANGE_V2,
        CTF_COLLATERAL_ADAPTER,
        NEG_RISK_CTF_COLLATERAL_ADAPTER,
    ]
}

pub fn predict_deposit_wallet(owner: Address) -> Result<Address, DispatchResponse> {
    let output = read_chain_eth_call(
        FACTORY,
        &DepositWalletFactory::implementationCall {}.abi_encode(),
        "factory implementation",
    )?;
    let implementation = DepositWalletFactory::implementationCall::abi_decode_returns(&output)
        .map_err(|err| error(-4, format!("factory implementation decode: {err}")))?;
    let mut wallet_id = [0u8; 32];
    wallet_id[12..].copy_from_slice(owner.as_slice());
    let output = read_chain_eth_call(
        FACTORY,
        &DepositWalletFactory::predictWalletAddressCall {
            implementation,
            walletId: wallet_id.into(),
        }
        .abi_encode(),
        "factory predictWalletAddress",
    )?;
    DepositWalletFactory::predictWalletAddressCall::abi_decode_returns(&output)
        .map_err(|err| error(-4, format!("factory predictWalletAddress decode: {err}")))
}

pub fn read_chain_eth_call(
    contract: Address,
    calldata: &[u8],
    label: &str,
) -> Result<Vec<u8>, DispatchResponse> {
    let (chain, _) = crate::runtime_config::chain().map_err(|err| error(-4, err))?;
    let result = petal::sdk::chain_read(
        &chain,
        "eth_call",
        &serde_json::json!([{
            "to": contract.to_checksum(None),
            "data": format!("0x{}", hex::encode(calldata)),
        }, "latest"])
        .to_string(),
    )
    .map_err(|err| sdk_error_with_context(label, err))?;
    let result: String =
        serde_json::from_str(&result).map_err(|err| error(-4, format!("{label} JSON: {err}")))?;
    let encoded = result
        .strip_prefix("0x")
        .ok_or_else(|| error(-4, format!("{label} result is not hex")))?;
    hex::decode(encoded).map_err(|err| error(-4, format!("{label} result hex: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_contract_reads_use_canonical_selectors() {
        assert_eq!(
            DepositWalletFactory::implementationCall::SELECTOR,
            [0x5c, 0x60, 0xda, 0x1b]
        );
        assert_eq!(
            DepositWalletFactory::predictWalletAddressCall::SELECTOR,
            [0x1f, 0x26, 0x47, 0x78]
        );
        assert_eq!(
            Erc20Reads::balanceOfCall::SELECTOR,
            [0x70, 0xa0, 0x82, 0x31]
        );
        assert_eq!(
            Erc20Reads::allowanceCall::SELECTOR,
            [0xdd, 0x62, 0xed, 0x3e]
        );
        assert_eq!(CtfReads::balanceOfCall::SELECTOR, [0x00, 0xfd, 0xd5, 0x8e]);
        assert_eq!(
            CtfReads::isApprovedForAllCall::SELECTOR,
            [0xe9, 0x85, 0xe9, 0xc5]
        );
    }

    #[test]
    fn clob_allowance_accepts_legacy_scalar_and_current_map() {
        assert_eq!(
            clob_collateral_allowance(&serde_json::json!({"allowance": "42"})),
            Some(U256::from(42))
        );
        assert_eq!(
            clob_collateral_allowance(&serde_json::json!({
                "allowances": {"first": "0", "second": "99"}
            })),
            Some(U256::from(99))
        );
    }
}
