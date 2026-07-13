use crate::prelude::*;

use crate::polymarket::signer::l2_headers;
use crate::polymarket::{Credentials, Result};
use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HttpRequest};
pub fn clob_l2_get_json(
    owner: Address,
    creds: &Credentials,
    path: &str,
    query: &[(&str, &str)],
) -> Result<serde_json::Value, DispatchResponse> {
    let timestamp = now_secs();
    let headers = l2_headers(
        owner,
        &creds.key,
        &creds.passphrase,
        &creds.secret,
        timestamp,
        "GET",
        path,
        "",
    )
    .map_err(|e| error(-4, e.to_string()))?;
    let url = url_with_query(
        &format!("{}{path}", crate::runtime_config::clob_url()),
        query,
    );
    let resp = petal::sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url,
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -4,
            format!(
                "CLOB account error (status {}): response body redacted ({} bytes)",
                resp.status,
                resp.body.len()
            ),
        ));
    }
    if resp.body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("json: {e}")))
}

pub fn clob_l2_post_json(
    owner: Address,
    creds: &Credentials,
    path: &str,
    body: &str,
) -> Result<serde_json::Value, DispatchResponse> {
    let timestamp = now_secs();
    let headers = l2_headers(
        owner,
        &creds.key,
        &creds.passphrase,
        &creds.secret,
        timestamp,
        "POST",
        path,
        body,
    )
    .map_err(|e| error(-4, e.to_string()))?;
    let resp = petal::sdk::http_fetch(
        &HttpRequest {
            method: "POST".into(),
            url: format!("{}{path}", crate::runtime_config::clob_url()),
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
            body: body.as_bytes().to_vec(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&resp.status) {
        let diagnostic = bounded_response_diagnostic(&resp.body);
        let code = if (400..500).contains(&resp.status) {
            // The CLOB received and definitively rejected the request. This is
            // not an ambiguous transport failure and must not be retried.
            -3
        } else {
            -4
        };
        return Err(error(
            code,
            format!("CLOB post error (status {}): {diagnostic}", resp.status),
        ));
    }
    if resp.body.iter().all(|b| b.is_ascii_whitespace()) {
        return if path == "/order" {
            Err(error(
                -4,
                "CLOB order response was empty; outcome is ambiguous",
            ))
        } else {
            Ok(serde_json::Value::Null)
        };
    }
    let value: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("json: {e}")))?;
    if path == "/order" {
        validate_order_post_response(&value)?;
    }
    Ok(value)
}

fn bounded_response_diagnostic(body: &[u8]) -> String {
    const MAX_CHARS: usize = 512;
    let text = String::from_utf8_lossy(body);
    let mut chars = text.trim().chars();
    let diagnostic: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{diagnostic}…")
    } else if diagnostic.is_empty() {
        "empty response body".into()
    } else {
        diagnostic
    }
}

fn validate_order_post_response(value: &serde_json::Value) -> Result<(), DispatchResponse> {
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        return Err(error(-3, "CLOB rejected the order in a 2xx response"));
    }
    let has_order_id = ["orderID", "orderId", "order_id", "id"].iter().any(|key| {
        value
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .is_some()
    });
    let recognized_status = value
        .get("status")
        .or_else(|| value.get("orderStatus"))
        .or_else(|| value.get("order_status"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "live" | "matched" | "unmatched" | "delayed" | "posted"
            )
        });
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(true)
        || has_order_id
        || recognized_status
    {
        Ok(())
    } else {
        Err(error(
            -4,
            "CLOB order response did not contain a definitive success marker; outcome is ambiguous",
        ))
    }
}

pub fn clob_l2_delete_json(
    owner: Address,
    creds: &Credentials,
    path: &str,
    body: &str,
) -> Result<serde_json::Value, DispatchResponse> {
    let timestamp = now_secs();
    let headers = l2_headers(
        owner,
        &creds.key,
        &creds.passphrase,
        &creds.secret,
        timestamp,
        "DELETE",
        path,
        body,
    )
    .map_err(|e| error(-4, e.to_string()))?;
    let resp = petal::sdk::http_fetch(
        &HttpRequest {
            method: "DELETE".into(),
            url: format!("{}{path}", crate::runtime_config::clob_url()),
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
            body: body.as_bytes().to_vec(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -4,
            format!("CLOB cancel error (status {})", resp.status),
        ));
    }
    if resp.body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(serde_json::json!({ "status": "empty" }));
    }
    serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("json: {e}")))
}

#[cfg(test)]
mod response_tests {
    use super::*;

    #[test]
    fn order_post_requires_positive_response_evidence() {
        assert!(
            validate_order_post_response(&serde_json::json!({
                "success": true,
                "orderID": "abc"
            }))
            .is_ok()
        );
        assert!(
            validate_order_post_response(&serde_json::json!({
                "success": false,
                "errorMsg": "bad order"
            }))
            .is_err()
        );
        assert!(validate_order_post_response(&serde_json::json!({})).is_err());
    }
}
