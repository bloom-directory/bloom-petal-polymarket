use crate::prelude::*;

use crate::polymarket::eip712::PUSD;
use crate::polymarket::order::parse_micro;
use crate::polymarket::{POLYGON, Result, validate_wallet_name};
use alloy::primitives::{Address, U256};
use petal::sdk::{DispatchResponse, EvmTransaction, HostStatus, HttpRequest, SdkError};
pub fn create_fund_request(wallet: &str, body: &[u8]) -> DispatchResponse {
    if let Err(e) = validate_wallet_name(wallet) {
        return error(-3, e.to_string());
    }
    let owner = match wallet_address(wallet) {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let deposit = match fundable_deposit_wallet(wallet, owner) {
        Ok(deposit) => deposit,
        Err(resp) => return resp,
    };
    let req: FundNewRequest = match serde_json::from_slice(body) {
        Ok(req) => req,
        Err(e) => return error(-3, format!("fund request JSON: {e}")),
    };
    if req.slippage_bps > 1000 {
        return error(-3, "slippage_bps too high (max 1000)");
    }
    if parse_micro(req.target_pusd.trim()).unwrap_or(0) == 0 {
        return error(-3, "target_pusd must be > 0");
    }
    if !positive_decimal_input(req.max_spend.trim()) {
        return error(-3, "max_spend must be > 0");
    }
    let id = next_id(&format!("fund/{wallet}/requests/"), ".json");
    let session = StoreFundSession {
        id: id.clone(),
        wallet: wallet.into(),
        target_pusd: req.target_pusd,
        max_spend: req.max_spend,
        from_token: req.from_token.unwrap_or_else(|| "native".into()),
        slippage_bps: req.slippage_bps,
        deposit_wallet: deposit.to_checksum(None),
        deposit_wallet_source: "live_factory_resolved".into(),
        status: "draft".into(),
        prepared_funding: None,
        review_intent: None,
        outbox_ids: Vec::new(),
        outbox_inspections: Vec::new(),
        next_transaction: 0,
        plan_md: None,
        approval: None,
        staging_transaction: None,
    };
    store_put_json(
        &format!("fund/{wallet}/requests/{id}.json"),
        &session,
        false,
    )
}

pub fn confirm_fund_request(wallet: &str, id: &str, body: &[u8]) -> DispatchResponse {
    let confirmation = match fund_confirmation(body) {
        Ok(confirmation) if confirmation.confirm => confirmation,
        Ok(_) | Err(()) => {
            return error(
                -3,
                "fund confirm requires confirm, y, or {\"confirm\":true}",
            );
        }
    };
    let _lock = match acquire_fund_lock(wallet, id) {
        Ok(lock) => lock,
        Err(resp) => return resp,
    };
    let key = format!("fund/{wallet}/requests/{id}.json");
    let mut session = match load_fund_session(wallet, id) {
        Ok(session) => session,
        Err(resp) => return resp,
    };
    if session.staging_transaction == Some(session.next_transaction)
        && session.outbox_ids.len() == session.next_transaction
    {
        return error(
            -4,
            "a prior outbox stage may have succeeded without its id becoming durable; refusing to restage automatically",
        );
    }
    let prepared = match session.prepared_funding.clone() {
        Some(prepared) => prepared,
        None => match prepare_funding(wallet, &session) {
            Ok(prepared) => {
                session.review_intent = Some(prepared.review_intent.clone());
                session.prepared_funding = Some(prepared);
                session.status = "prepared".into();
                return store_put_json(&key, &session, false);
            }
            Err(resp) => return resp,
        },
    };
    if session.next_transaction >= prepared.transactions.len() {
        return store_put_json(&key, &session, false);
    }
    if let Some(outbox_id) = session.outbox_ids.get(session.next_transaction).cloned() {
        let inspection = match petal::sdk::tx_inspect(wallet, "polygon", &outbox_id) {
            Ok(inspection) => inspection,
            Err(err) => return sdk_error(err),
        };
        let record = serde_json::json!({
            "outbox_id": inspection.outbox_id,
            "state": inspection.state,
            "tx_hash": inspection.tx_hash,
            "receipt": inspection.receipt_json.as_deref().and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok()),
        });
        if let Some(existing) = session
            .outbox_inspections
            .iter_mut()
            .find(|value| value.get("outbox_id") == record.get("outbox_id"))
        {
            *existing = record;
        } else {
            session.outbox_inspections.push(record);
        }
        match inspection.state.as_str() {
            "success" => {
                session.next_transaction += 1;
                session.approval = None;
                session.status = if session.next_transaction == prepared.transactions.len() {
                    "complete"
                } else {
                    "transaction_confirmed"
                }
                .into();
                return store_put_json(&key, &session, false);
            }
            "reverted" | "failed" | "cancelled" => {
                session.status = format!("transaction_{}", inspection.state);
                let _ = store_put_json(&key, &session, false);
                return error(-4, "funding transaction failed; refusing automatic retry");
            }
            "sent" => {
                session.status = "awaiting_confirmation".into();
                return store_put_json(&key, &session, false);
            }
            _ => {}
        }
    }
    let transaction = &prepared.transactions[session.next_transaction];
    if session.outbox_ids.len() == session.next_transaction {
        session.staging_transaction = Some(session.next_transaction);
        session.status = "staging_started".into();
        if let DispatchResponse::Error { .. } = store_put_json(&key, &session, false) {
            return error(-4, "failed to persist outbox staging marker");
        }
        let staged = match petal::sdk::tx_stage(&EvmTransaction {
            wallet: wallet.into(),
            chain: "polygon".into(),
            to: transaction.to.clone(),
            value_wei: transaction.value_wei.clone(),
            data_hex: transaction.data_hex.clone(),
            nonce: None,
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
        }) {
            Ok(staged) => staged,
            Err(err @ SdkError::Host(HostStatus::Denied | HostStatus::Invalid)) => {
                session.staging_transaction = None;
                session.status = "prepared".into();
                if let DispatchResponse::Error { .. } = store_put_json(&key, &session, false) {
                    return error(-4, "failed to clear rejected outbox staging marker");
                }
                return sdk_error(err);
            }
            Err(err) => return sdk_error(err),
        };
        session.outbox_ids.push(staged.outbox_id);
        session.staging_transaction = None;
        session.plan_md = Some(staged.plan_md);
        session.status = "staged".into();
        if let DispatchResponse::Error { .. } = store_put_json(&key, &session, false) {
            return error(
                -4,
                "staged outbox id could not be persisted; refusing restaging",
            );
        }
        // The authoritative outbox plan (including simulation and warnings) is
        // now readable. Require a distinct write before confirmation.
        return DispatchResponse::Write;
    }
    let outbox_id = &session.outbox_ids[session.next_transaction];
    let outcome = match petal::sdk::tx_confirm(
        wallet,
        "polygon",
        outbox_id,
        confirmation.acknowledge_warnings,
    ) {
        Ok(outcome) => outcome,
        Err(err) => return sdk_error(err),
    };
    session.status = if outcome.approval.is_some() {
        "approval_required"
    } else {
        "awaiting_confirmation"
    }
    .into();
    session.plan_md = Some(outcome.plan_md);
    session.approval = outcome.approval.map(|approval| {
        serde_json::json!({
            "action_id": approval.action_id,
            "ceremony_url": approval.ceremony_url,
            "expires_ms": approval.expires_ms,
            "prepared_artifact_digest": prepared.digest(),
            "retry_state": "approval_required",
            "operation": "fund",
        })
    });
    store_put_json(&key, &session, false)
}

pub fn read_review(wallet: &str, id: &str) -> DispatchResponse {
    match load_fund_session(wallet, id) {
        Ok(session) => match session.review_intent {
            Some(review) => petal::read_json_value(&review),
            None => error(-1, "funding transaction has not been prepared"),
        },
        Err(resp) => resp,
    }
}

pub fn read_approval(wallet: &str, id: &str) -> DispatchResponse {
    match load_fund_session(wallet, id) {
        Ok(session) => match session.approval {
            Some(approval) => petal::read_json_value(&approval),
            None => error(-1, "no funding approval is pending"),
        },
        Err(resp) => resp,
    }
}

fn prepare_funding(
    wallet: &str,
    session: &StoreFundSession,
) -> Result<PreparedFunding, DispatchResponse> {
    let owner = wallet_address(wallet)?;
    let deposit = session
        .deposit_wallet
        .parse::<Address>()
        .map_err(|err| error(-4, format!("corrupt deposit wallet: {err}")))?;
    let target = parse_micro(&session.target_pusd)
        .map(U256::from)
        .map_err(polymarket_error)?;
    let current = read_chain_erc20_balance(PUSD, deposit)?;
    let missing = target.saturating_sub(current);
    if missing.is_zero() {
        return Err(error(-3, "deposit wallet already meets the pUSD target"));
    }
    if session.from_token.eq_ignore_ascii_case("pusd") {
        let max_spend = parse_decimal_units(&session.max_spend, 6)?;
        validate_direct_pusd_funding(missing, max_spend, read_chain_erc20_balance(PUSD, owner)?)?;
        return Ok(direct_pusd(wallet, session, deposit, missing));
    }
    prepare_enso(wallet, session, owner, deposit, missing)
}

fn direct_pusd(
    wallet: &str,
    session: &StoreFundSession,
    deposit: Address,
    missing: U256,
) -> PreparedFunding {
    let transaction = PreparedEvmTransaction {
        purpose: "direct_pusd_transfer".into(),
        to: PUSD.to_checksum(None),
        value_wei: "0".into(),
        data_hex: erc20_transfer_calldata(deposit, missing),
    };
    PreparedFunding {
        review_intent: serde_json::json!({
            "operation": "polymarket_fund",
            "wallet": wallet,
            "chain": "polygon",
            "from_token": "pUSD",
            "recipient": deposit.to_checksum(None),
            "amount_pusd_micro": missing.to_string(),
            "max_spend": session.max_spend,
            "slippage_bps": session.slippage_bps,
            "transactions": [transaction.clone()],
        }),
        transactions: vec![transaction],
    }
}

fn prepare_enso(
    wallet: &str,
    session: &StoreFundSession,
    owner: Address,
    deposit: Address,
    missing: U256,
) -> Result<PreparedFunding, DispatchResponse> {
    let api_key = crate::account_views::load_enso_api_key()?;
    let trusted_router = crate::account_views::load_enso_router()?;
    let native = matches!(
        session.from_token.to_ascii_lowercase().as_str(),
        "native" | "pol" | "matic"
    );
    let token_in = if native {
        alloy::primitives::address!("EeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE")
    } else {
        session.from_token.parse::<Address>().map_err(|err| {
            error(
                -3,
                format!("from_token must be native or an address: {err}"),
            )
        })?
    };
    let decimals = if native {
        18
    } else {
        read_chain_erc20_decimals(token_in)?
    };
    let max_spend = parse_decimal_units(&session.max_spend, decimals)?;
    if max_spend.is_zero() {
        return Err(error(-3, "max_spend must be greater than zero"));
    }
    let input_balance = if native {
        read_chain_native_balance(owner)?
    } else {
        read_chain_erc20_balance(token_in, owner)?
    };
    if input_balance < max_spend {
        return Err(error(-3, "input balance is below max_spend"));
    }
    let common = vec![
        ("fromAddress".into(), owner.to_checksum(None)),
        ("chainId".into(), POLYGON.to_string()),
        ("tokenIn".into(), token_in.to_checksum(None)),
        ("tokenOut".into(), PUSD.to_checksum(None)),
        ("slippage".into(), session.slippage_bps.to_string()),
        ("routingStrategy".into(), "router".into()),
        ("receiver".into(), deposit.to_checksum(None)),
    ];
    let mut quote_params = common.clone();
    quote_params.push(("amountIn".into(), max_spend.to_string()));
    let quote = enso_get("/api/v1/shortcuts/quote", &quote_params, &api_key)?;
    let output_at_max = json_u256(
        quote
            .get("amountOut")
            .ok_or_else(|| error(-4, "Enso quote missing amountOut"))?,
    )?;
    if output_at_max < missing {
        return Err(error(-3, "max_spend cannot buy the missing pUSD amount"));
    }
    let required_input = funding_required_input(max_spend, missing, output_at_max);
    let mut route_params = common;
    route_params.push(("amountIn".into(), required_input.to_string()));
    let route = enso_get("/api/v1/shortcuts/route", &route_params, &api_key)?;
    let output = json_u256(
        route
            .get("amountOut")
            .ok_or_else(|| error(-4, "Enso route missing amountOut"))?,
    )?;
    if output < missing {
        return Err(error(-3, "Enso route output is below missing pUSD"));
    }
    if route
        .get("route")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|hop| hop.get("destinationChainId"))
        .filter_map(|value| json_u256(value).ok())
        .any(|chain| chain != U256::from(POLYGON))
    {
        return Err(error(-3, "cross-chain Enso routes are forbidden"));
    }
    let tx = route
        .get("tx")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| error(-4, "Enso route missing tx"))?;
    let to = tx
        .get("to")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, "Enso route missing tx.to"))?
        .parse::<Address>()
        .map_err(|err| error(-4, format!("Enso router address: {err}")))?;
    let from = tx
        .get("from")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, "Enso route missing tx.from"))?
        .parse::<Address>()
        .map_err(|err| error(-4, format!("Enso sender address: {err}")))?;
    if to != trusted_router || from != owner {
        return Err(error(-3, "Enso route sender or router is invalid"));
    }
    let data = tx
        .get("data")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| error(-4, "Enso route missing calldata"))?
        .to_ascii_lowercase();
    let decoded = hex::decode(
        data.strip_prefix("0x")
            .ok_or_else(|| error(-4, "Enso calldata is not hex"))?,
    )
    .map_err(|err| error(-4, format!("Enso calldata: {err}")))?;
    if !decoded
        .windows(20)
        .any(|window| window == deposit.as_slice())
    {
        return Err(error(-3, "Enso calldata does not bind the deposit wallet"));
    }
    let value = tx
        .get("value")
        .map(json_u256)
        .transpose()?
        .unwrap_or_default();
    if (native && value != required_input) || (!native && !value.is_zero()) {
        return Err(error(
            -3,
            "Enso route native value does not match its input",
        ));
    }
    let mut transactions = Vec::new();
    if !native && read_chain_erc20_allowance(token_in, owner, to)? < required_input {
        transactions.push(PreparedEvmTransaction {
            purpose: "erc20_exact_approval".into(),
            to: token_in.to_checksum(None),
            value_wei: "0".into(),
            data_hex: erc20_approve_calldata(to, required_input),
        });
    }
    transactions.push(PreparedEvmTransaction {
        purpose: "enso_swap".into(),
        to: to.to_checksum(None),
        value_wei: value.to_string(),
        data_hex: data,
    });
    let quote_digest = blake3_hex(
        &serde_json::to_vec(&quote).map_err(|err| error(-4, format!("Enso quote JSON: {err}")))?,
    );
    let route_digest = blake3_hex(
        &serde_json::to_vec(&route).map_err(|err| error(-4, format!("Enso route JSON: {err}")))?,
    );
    Ok(PreparedFunding {
        review_intent: serde_json::json!({
            "operation": "polymarket_fund",
            "wallet": wallet,
            "owner": owner.to_checksum(None),
            "deposit_wallet": deposit.to_checksum(None),
            "input_token": token_in.to_checksum(None),
            "input_decimals": decimals,
            "input_balance": input_balance.to_string(),
            "max_spend": max_spend.to_string(),
            "required_input": required_input.to_string(),
            "missing_pusd_micro": missing.to_string(),
            "route_output_pusd_micro": output.to_string(),
            "slippage_bps": session.slippage_bps,
            "quote_digest": quote_digest,
            "route_digest": route_digest,
            "router": to.to_checksum(None),
            "transactions": transactions,
        }),
        transactions,
    })
}

fn enso_get(
    path: &str,
    params: &[(String, String)],
    api_key: &str,
) -> Result<serde_json::Value, DispatchResponse> {
    let borrowed: Vec<_> = params
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let response = petal::sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url: url_with_query(&format!("https://api.enso.finance{path}"), &borrowed),
            headers: vec![("authorization".into(), format!("Bearer {api_key}"))],
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&response.status) {
        return Err(error(
            -4,
            format!(
                "Enso request failed with status {} (body redacted, {} bytes)",
                response.status,
                response.body.len()
            ),
        ));
    }
    serde_json::from_slice(&response.body)
        .map_err(|err| error(-4, format!("Enso response JSON: {err}")))
}

fn funding_required_input(max_spend: U256, missing: U256, output_at_max: U256) -> U256 {
    if output_at_max.is_zero() {
        return max_spend;
    }
    max_spend
        .saturating_mul(missing)
        .checked_div(output_at_max)
        .unwrap_or(max_spend)
        .saturating_mul(U256::from(102u8))
        .checked_div(U256::from(100u8))
        .unwrap_or(max_spend)
        .min(max_spend)
}

fn read_chain_native_balance(holder: Address) -> Result<U256, DispatchResponse> {
    let result = petal::sdk::chain_read(
        "polygon",
        "eth_getBalance",
        &serde_json::json!([holder.to_checksum(None), "latest"]).to_string(),
    )
    .map_err(sdk_error)?;
    parse_chain_quantity(&result, "native balance")
}

fn read_chain_erc20_decimals(token: Address) -> Result<u8, DispatchResponse> {
    let value = read_chain_eth_call_u256(token, &[0x31, 0x3c, 0xe5, 0x67], "ERC20 decimals")?;
    u8::try_from(value).map_err(|_| error(-4, "ERC20 decimals exceed 255"))
}

fn read_chain_eth_call_u256(
    contract: Address,
    calldata: &[u8],
    field: &str,
) -> Result<U256, DispatchResponse> {
    let result = petal::sdk::chain_read(
        "polygon",
        "eth_call",
        &serde_json::json!([{
            "to": contract.to_checksum(None),
            "data": format!("0x{}", hex::encode(calldata)),
        }, "latest"])
        .to_string(),
    )
    .map_err(sdk_error)?;
    parse_chain_quantity(&result, field)
}

fn parse_chain_quantity(result_json: &str, field: &str) -> Result<U256, DispatchResponse> {
    let result: String = serde_json::from_str(result_json)
        .map_err(|err| error(-4, format!("{field} JSON: {err}")))?;
    let hex = result
        .strip_prefix("0x")
        .ok_or_else(|| error(-4, format!("{field} is not hex")))?;
    U256::from_str_radix(if hex.is_empty() { "0" } else { hex }, 16)
        .map_err(|err| error(-4, format!("{field}: {err}")))
}

fn erc20_approve_calldata(spender: Address, amount: U256) -> String {
    let mut bytes = vec![0x09, 0x5e, 0xa7, 0xb3];
    let mut encoded_spender = [0u8; 32];
    encoded_spender[12..].copy_from_slice(spender.as_slice());
    bytes.extend_from_slice(&encoded_spender);
    bytes.extend_from_slice(&amount.to_be_bytes::<32>());
    format!("0x{}", hex::encode(bytes))
}

fn erc20_transfer_calldata(to: Address, amount: U256) -> String {
    let mut bytes = vec![0xa9, 0x05, 0x9c, 0xbb];
    let mut recipient = [0u8; 32];
    recipient[12..].copy_from_slice(to.as_slice());
    bytes.extend_from_slice(&recipient);
    bytes.extend_from_slice(&amount.to_be_bytes::<32>());
    format!("0x{}", hex::encode(bytes))
}

fn fund_confirmation(body: &[u8]) -> Result<FundConfirmRequest, ()> {
    let Ok(text) = std::str::from_utf8(body) else {
        return Err(());
    };
    if matches!(
        text.trim().to_ascii_lowercase().as_str(),
        "confirm" | "y" | "yes"
    ) {
        return Ok(FundConfirmRequest {
            confirm: true,
            acknowledge_warnings: false,
        });
    }
    serde_json::from_str(text).map_err(|_| ())
}

struct FundLock {
    key: String,
    expected: Vec<u8>,
}

impl Drop for FundLock {
    fn drop(&mut self) {
        let _ = petal::sdk::store_del_if_value(&self.key, &self.expected);
    }
}

fn acquire_fund_lock(wallet: &str, id: &str) -> Result<FundLock, DispatchResponse> {
    let key = format!("fund/{wallet}/requests/{id}.lock");
    for attempt in 0..2 {
        let body = trade_lock_body(wallet, id)?;
        match petal::sdk::store_put_new(&key, &body, false) {
            Ok(()) => {
                return Ok(FundLock {
                    key,
                    expected: body,
                });
            }
            Err(SdkError::Host(HostStatus::Denied)) if attempt == 0 => {
                let Some(stale) = trade_lock_stale_bytes(&key) else {
                    return Err(error(
                        -3,
                        "another funding operation holds this session lock",
                    ));
                };
                match petal::sdk::store_del_if_value(&key, &stale) {
                    Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => continue,
                    Err(SdkError::Host(HostStatus::Denied)) => {
                        return Err(error(-3, "the funding session lock was refreshed"));
                    }
                    Err(err) => return Err(sdk_error(err)),
                }
            }
            Err(SdkError::Host(HostStatus::Denied)) => {
                return Err(error(
                    -3,
                    "another funding operation holds this session lock",
                ));
            }
            Err(err) => return Err(sdk_error(err)),
        }
    }
    Err(error(
        -3,
        "another funding operation holds this session lock",
    ))
}

fn validate_direct_pusd_funding(
    missing: U256,
    max_spend: U256,
    owner_balance: U256,
) -> Result<(), DispatchResponse> {
    if missing > max_spend {
        return Err(error(-3, "missing pUSD exceeds max_spend"));
    }
    if owner_balance < missing {
        return Err(error(
            -3,
            "owner pUSD balance is below the required transfer",
        ));
    }
    Ok(())
}

fn parse_decimal_units(value: &str, decimals: u8) -> Result<U256, DispatchResponse> {
    let (whole, fraction) = value.trim().split_once('.').unwrap_or((value.trim(), ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > decimals as usize
    {
        return Err(error(-3, "amount has invalid decimal precision"));
    }
    let whole = whole
        .parse::<U256>()
        .map_err(|err| error(-3, format!("amount: {err}")))?;
    let mut fraction = fraction.to_string();
    fraction.extend(core::iter::repeat_n(
        '0',
        decimals as usize - fraction.len(),
    ));
    let fraction = if fraction.is_empty() {
        U256::ZERO
    } else {
        fraction
            .parse::<U256>()
            .map_err(|err| error(-3, format!("amount: {err}")))?
    };
    whole
        .checked_mul(U256::from(10u8).pow(U256::from(decimals)))
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| error(-3, "amount overflows uint256"))
}

fn positive_decimal_input(value: &str) -> bool {
    let mut saw_digit = false;
    let mut saw_nonzero = false;
    let mut saw_dot = false;
    for byte in value.bytes() {
        match byte {
            b'0'..=b'9' => {
                saw_digit = true;
                saw_nonzero |= byte != b'0';
            }
            b'.' if !saw_dot => saw_dot = true,
            _ => return false,
        }
    }
    saw_digit && saw_nonzero && !value.starts_with('.') && !value.ends_with('.')
}

fn json_u256(value: &serde_json::Value) -> Result<U256, DispatchResponse> {
    let raw = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|value| value.to_string()))
        .ok_or_else(|| error(-4, "response integer is invalid"))?;
    if let Some(hex) = raw.strip_prefix("0x") {
        U256::from_str_radix(hex, 16).map_err(|err| error(-4, format!("hex integer: {err}")))
    } else {
        raw.parse::<U256>()
            .map_err(|err| error(-4, format!("integer: {err}")))
    }
}

pub fn load_fund_session(wallet: &str, id: &str) -> Result<StoreFundSession, DispatchResponse> {
    if let Err(e) = validate_wallet_name(wallet) {
        return Err(error(-3, e.to_string()));
    }
    let Some(bytes) = store_get(&format!("fund/{wallet}/requests/{id}.json")) else {
        return Err(error(-1, "not found"));
    };
    serde_json::from_slice(&bytes).map_err(|e| error(-4, format!("corrupt fund request: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_transfer_calldata_is_canonical() {
        let recipient = Address::repeat_byte(0x11);
        let calldata = erc20_transfer_calldata(recipient, U256::from(7u8));
        assert!(calldata.starts_with("0xa9059cbb"));
        assert_eq!(calldata.len(), 2 + (4 + 32 + 32) * 2);
        assert!(calldata.contains(&hex::encode(recipient.as_slice())));
    }

    #[test]
    fn direct_pusd_enforces_spend_cap_and_balance() {
        let missing = U256::from(10_000_000u64);
        assert!(validate_direct_pusd_funding(missing, U256::from(1_000_000u64), missing).is_err());
        assert!(validate_direct_pusd_funding(missing, missing, U256::from(9_000_000u64)).is_err());
        assert!(validate_direct_pusd_funding(missing, missing, missing).is_ok());
    }

    #[test]
    fn exact_approval_calldata_is_canonical() {
        let spender = Address::repeat_byte(0x22);
        let calldata = erc20_approve_calldata(spender, U256::from(7u8));
        assert!(calldata.starts_with("0x095ea7b3"));
        assert_eq!(calldata.len(), 2 + (4 + 32 + 32) * 2);
        assert!(calldata.contains(&hex::encode(spender.as_slice())));
    }

    #[test]
    fn quote_sizing_is_capped_and_buffered() {
        assert_eq!(
            funding_required_input(U256::from(100u8), U256::from(50u8), U256::from(100u8)),
            U256::from(51u8)
        );
        assert_eq!(
            funding_required_input(U256::from(100u8), U256::from(200u8), U256::from(100u8)),
            U256::from(100u8)
        );
    }

    #[test]
    fn funding_confirmation_is_explicit() {
        assert!(fund_confirmation(b"confirm").unwrap().confirm);
        assert!(fund_confirmation(br#"{"confirm":true}"#).unwrap().confirm);
        assert!(fund_confirmation(b"").is_err());
        assert!(!fund_confirmation(br#"{"confirm":false}"#).unwrap().confirm);
        assert!(
            fund_confirmation(br#"{"confirm":true,"acknowledge_warnings":true}"#)
                .unwrap()
                .acknowledge_warnings
        );
    }

    #[test]
    fn decimal_units_are_bounded() {
        assert_eq!(
            parse_decimal_units("1.25", 6).unwrap(),
            U256::from(1_250_000u64)
        );
        assert!(parse_decimal_units("1.0000001", 6).is_err());
        assert!(parse_decimal_units("-1", 6).is_err());
        assert!(positive_decimal_input("0.0000001"));
        assert!(!positive_decimal_input("0.0"));
        assert!(!positive_decimal_input("1."));
    }
}
