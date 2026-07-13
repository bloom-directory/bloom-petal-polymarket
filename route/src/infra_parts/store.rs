use crate::prelude::*;

use crate::polymarket::Result;
use petal::sdk::{DispatchResponse, HostStatus, SdkError};
use serde::Serialize;
pub fn read_store(key: &str) -> DispatchResponse {
    match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => DispatchResponse::Read(bytes),
        Err(SdkError::Host(HostStatus::NotFound)) => error(-1, "not found"),
        Err(e) => sdk_error(e),
    }
}

pub fn acquire_trade_lock(
    wallet: &str,
    draft_id: &str,
) -> Result<StoreTradeLock, DispatchResponse> {
    let key = format!("trade/{wallet}/.lock");
    for attempt in 0..2 {
        let bytes = trade_lock_body(wallet, draft_id)?;
        match petal::sdk::store_put_new(&key, &bytes, false) {
            Ok(()) => {
                return Ok(StoreTradeLock {
                    key,
                    expected: bytes,
                });
            }
            Err(SdkError::Host(HostStatus::Denied)) if attempt == 0 => {
                let Some(stale_bytes) = trade_lock_stale_bytes(&key) else {
                    return Err(error(
                        -3,
                        format!("another trade operation holds the lock for wallet '{wallet}'"),
                    ));
                };
                match petal::sdk::store_del_if_value(&key, &stale_bytes) {
                    Ok(()) | Err(SdkError::Host(HostStatus::NotFound)) => continue,
                    Err(SdkError::Host(HostStatus::Denied)) => {
                        return Err(error(
                            -3,
                            format!(
                                "another trade operation refreshed the lock for wallet '{wallet}'"
                            ),
                        ));
                    }
                    Err(e) => return Err(sdk_error(e)),
                }
            }
            Err(SdkError::Host(HostStatus::Denied)) => {
                return Err(error(
                    -3,
                    format!("another trade operation holds the lock for wallet '{wallet}'"),
                ));
            }
            Err(e) => return Err(sdk_error(e)),
        }
    }
    Err(error(
        -3,
        format!("another trade operation holds the lock for wallet '{wallet}'"),
    ))
}

pub fn trade_lock_body(wallet: &str, draft_id: &str) -> Result<Vec<u8>, DispatchResponse> {
    let mut token = [0u8; 16];
    let random = petal::sdk::random_bytes(token.len())
        .map_err(|e| error(-4, format!("trade lock random token: {}", e.message())))?;
    token.copy_from_slice(&random);
    let body = serde_json::json!({
        "wallet": wallet,
        "draft_id": draft_id,
        "acquired_ms": now_millis(),
        "token": hex::encode(token)
    });
    serde_json::to_vec(&body).map_err(|e| error(-4, format!("json: {e}")))
}

pub fn trade_lock_stale_bytes(key: &str) -> Option<Vec<u8>> {
    match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => {
            let stale = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("acquired_ms").and_then(serde_json::Value::as_u64))
                .map(|acquired| now_millis().saturating_sub(acquired as u128) > TRADE_LOCK_STALE_MS)
                .unwrap_or(true);
            stale.then_some(bytes)
        }
        Err(_) => None,
    }
}

pub struct StoreTradeLock {
    key: String,
    expected: Vec<u8>,
}

impl Drop for StoreTradeLock {
    fn drop(&mut self) {
        let _ = petal::sdk::store_del_if_value(&self.key, &self.expected);
    }
}

pub fn store_get(key: &str) -> Option<Vec<u8>> {
    petal::sdk::store_get(key, MAX_STORE_BYTES).ok()
}

pub fn store_put_json<T: Serialize>(key: &str, value: &T, secret: bool) -> DispatchResponse {
    let bytes = match serde_json::to_vec_pretty(value) {
        Ok(bytes) => bytes,
        Err(e) => return error(-4, format!("json: {e}")),
    };
    match petal::sdk::store_put(key, &bytes, secret) {
        Ok(()) => DispatchResponse::Write,
        Err(e) => sdk_error(e),
    }
}

pub fn store_trade_receipt(
    wallet: &str,
    id: &str,
    receipt: &StoreTradeReceipt,
) -> DispatchResponse {
    let audit_resp = append_trade_audit(
        wallet,
        "receipt_written",
        serde_json::json!({
            "draft_id": id,
            "clob_status": receipt.clob_status,
            "amount_microusd": receipt.amount_microusd,
        }),
    );
    if let DispatchResponse::Error { .. } = audit_resp {
        return audit_resp;
    }
    store_put_json(
        &format!("trade/{wallet}/receipts/{id}/receipt.json"),
        receipt,
        false,
    )
}

pub fn append_trade_audit(
    wallet: &str,
    event: &str,
    details: serde_json::Value,
) -> DispatchResponse {
    let key = format!("trade/{wallet}/audit.jsonl");
    let mut text = match petal::sdk::store_get(&key, MAX_STORE_BYTES) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(SdkError::Host(HostStatus::NotFound)) => String::new(),
        Err(e) => return sdk_error(e),
    };
    let line = serde_json::json!({
        "ts_ms": now_millis(),
        "event": event,
        "details": details,
    });
    text.push_str(&line.to_string());
    text.push('\n');
    match petal::sdk::store_put(&key, text.as_bytes(), false) {
        Ok(()) => DispatchResponse::Write,
        Err(e) => sdk_error(e),
    }
}
