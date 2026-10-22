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

/// The package-global record of builder attribution changes.
pub const BUILDER_SETTINGS_AUDIT_KEY: &str = "settings/builder-audit.jsonl";

fn append_audit_line(key: &str, event: &str, details: serde_json::Value) -> DispatchResponse {
    let mut text = match petal::sdk::store_get(key, MAX_STORE_BYTES) {
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
    match petal::sdk::store_put(key, text.as_bytes(), false) {
        Ok(()) => DispatchResponse::Write,
        Err(e) => sdk_error(e),
    }
}

pub fn append_trade_audit(
    wallet: &str,
    event: &str,
    details: serde_json::Value,
) -> DispatchResponse {
    append_audit_line(&format!("trade/{wallet}/audit.jsonl"), event, details)
}

/// Records a change to the builder code or its fee ceiling.
///
/// Those two settings decide who is paid and how much, and both are ordinary
/// agent writes. The per-order ceremony is what stops an undisclosed fee
/// reaching a signature; this is what makes the change itself visible
/// afterwards, named and timed rather than inferred from a later balance.
pub fn append_builder_settings_audit(event: &str, details: serde_json::Value) -> DispatchResponse {
    let text = match petal::sdk::store_get(BUILDER_SETTINGS_AUDIT_KEY, MAX_STORE_BYTES) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(SdkError::Host(HostStatus::NotFound)) => String::new(),
        Err(e) => return sdk_error(e),
    };
    let body = builder_settings_audit_body(&text, event, details, now_millis());
    match petal::sdk::store_put(BUILDER_SETTINGS_AUDIT_KEY, body.as_bytes(), false) {
        Ok(()) => DispatchResponse::Write,
        Err(e) => sdk_error(e),
    }
}

/// Lines kept in the settings record.
///
/// A bound is not a nicety. `store_get` fails with `BufferTooSmall` once a
/// value passes the limit it is given, and nothing rotates this key, so an
/// unbounded log would eventually make every later setting change fail to
/// record *after it had already applied*, permanently, with no route able to
/// prune it. At roughly 140 bytes a line this stays well under the 1 MiB read.
const MAX_BUILDER_SETTINGS_AUDIT_LINES: usize = 2_000;

fn audit_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// The record's next state: the change appended, sequenced, and the oldest
/// lines dropped once the bound is reached.
///
/// Pure, and taking the clock as an argument, because the store read and write
/// around it have no double in this suite — this is the part that can be held
/// to behaving.
fn builder_settings_audit_body(
    text: &str,
    event: &str,
    details: serde_json::Value,
    ts_ms: u128,
) -> String {
    let mut lines = audit_lines(text);
    // Sequenced, because a window onto the end of a log can be emptied by
    // filling it: twenty more changes push a repointed code out of view. The
    // numbers survive trimming, so a tail starting at 68 says plainly that 67
    // changes came before it rather than reading as the whole history.
    let seq = audit_last_seq(&lines).saturating_add(1);
    let line = serde_json::json!({
        "seq": seq,
        "ts_ms": ts_ms,
        "event": event,
        "details": details,
    })
    .to_string();
    lines.push(&line);
    let mut body = lines[lines.len().saturating_sub(MAX_BUILDER_SETTINGS_AUDIT_LINES)..].join("\n");
    body.push('\n');
    body
}

fn audit_last_seq(lines: &[&str]) -> u64 {
    lines
        .last()
        .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .and_then(|line| line.get("seq").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

fn audit_tail_from_text(text: &str, limit: usize) -> Vec<serde_json::Value> {
    let lines = audit_lines(text);
    lines
        .iter()
        .skip(lines.len().saturating_sub(limit))
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The most recent builder attribution changes, oldest first, or `None` when
/// the record exists but could not be read.
///
/// The distinction is the point. Reporting an unreadable log as an empty one
/// renders "nothing has ever changed" and "the history is gone" identically,
/// which is the more comfortable of the two readings and the wrong one.
pub fn builder_settings_audit_tail(limit: usize) -> Option<Vec<serde_json::Value>> {
    let bytes = match petal::sdk::store_get(BUILDER_SETTINGS_AUDIT_KEY, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => return Some(Vec::new()),
        Err(_) => return None,
    };
    Some(audit_tail_from_text(
        &String::from_utf8_lossy(&bytes),
        limit,
    ))
}

#[cfg(test)]
mod builder_settings_audit_tests {
    use super::*;

    fn append(text: &str, event: &str) -> String {
        builder_settings_audit_body(text, event, serde_json::json!({}), 7)
    }

    /// A window onto the end of the log can be emptied by filling it, so the
    /// numbers have to say how much came before. They must survive the trim
    /// that keeps the log readable at all.
    #[test]
    fn changes_are_sequenced_and_the_numbers_survive_trimming() {
        let mut text = String::new();
        for _ in 0..(MAX_BUILDER_SETTINGS_AUDIT_LINES + 25) {
            text = append(&text, "builder_code_set");
        }
        let lines = audit_lines(&text);
        assert_eq!(
            lines.len(),
            MAX_BUILDER_SETTINGS_AUDIT_LINES,
            "the record must stay bounded, or it eventually stops recording at all"
        );
        assert_eq!(
            audit_last_seq(&lines),
            (MAX_BUILDER_SETTINGS_AUDIT_LINES + 25) as u64,
            "the sequence must keep counting through a trim"
        );
        let tail = audit_tail_from_text(&text, 20);
        assert_eq!(tail.len(), 20);
        let seq_of =
            |entry: &serde_json::Value| entry.get("seq").and_then(serde_json::Value::as_u64);
        // The tail must be the LAST 20, not the first 20: the recent changes
        // are the point, and a window onto the oldest ones would hide every
        // change made since.
        let total = (MAX_BUILDER_SETTINGS_AUDIT_LINES + 25) as u64;
        assert_eq!(seq_of(tail.last().unwrap()), Some(total), "{tail:?}");
        assert_eq!(seq_of(&tail[0]), Some(total - 19), "{tail:?}");
    }

    #[test]
    fn a_tail_skips_blank_and_malformed_lines_and_reads_oldest_first() {
        let text = format!(
            "\n{{not json\n{}\n",
            append("", "builder_fee_ceiling_set").trim()
        );
        let tail = audit_tail_from_text(&text, 20);
        assert_eq!(tail.len(), 1, "{tail:?}");
        assert_eq!(
            tail[0].get("event").and_then(serde_json::Value::as_str),
            Some("builder_fee_ceiling_set")
        );

        let two = append(&append("", "builder_code_set"), "builder_code_cleared");
        let tail = audit_tail_from_text(&two, 20);
        assert_eq!(
            tail.iter()
                .filter_map(|line| line.get("seq").and_then(serde_json::Value::as_u64))
                .collect::<Vec<_>>(),
            vec![1, 2],
            "oldest first"
        );
    }
}
