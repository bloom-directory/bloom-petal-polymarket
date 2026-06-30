use crate::*;

use crate::bloom_petal_sdk::{DispatchResponse, HttpRequest};
use crate::polymarket::{Credentials, Result};
use crate::signer::l2_headers;
use alloy::primitives::Address;
pub(crate) fn clob_l2_get_json(
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
    let url = url_with_query(&format!("{CLOB}{path}"), query);
    let resp = bloom_petal_sdk::http_fetch(
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

pub(crate) fn clob_l2_post_json(
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
    let resp = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: "POST".into(),
            url: format!("{CLOB}{path}"),
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
            format!("CLOB post error (status {})", resp.status),
        ));
    }
    if resp.body.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(serde_json::json!({ "status": "posted" }));
    }
    serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("json: {e}")))
}

pub(crate) fn clob_l2_delete_json(
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
    let resp = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: "DELETE".into(),
            url: format!("{CLOB}{path}"),
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
