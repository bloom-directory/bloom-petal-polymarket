use crate::prelude::*;

use crate::polymarket::{Credentials, Result};
use petal::sdk::{DispatchResponse, HttpRequest};
pub fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, DispatchResponse> {
    let resp = http("GET", url, &[], Vec::new())?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -4,
            format!(
                "polymarket api error (status {}): {}",
                resp.status,
                String::from_utf8_lossy(&resp.body)
            ),
        ));
    }
    serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("json: {e}")))
}

pub fn clob_auth_request(
    method: &str,
    path: &str,
    headers: &[(&str, String)],
) -> Result<Credentials, DispatchResponse> {
    let resp = petal::sdk::http_fetch(
        &HttpRequest {
            method: method.into(),
            url: format!("{}{path}", crate::runtime_config::clob_url()),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).into(), value.clone()))
                .collect(),
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&resp.status) {
        return Err(error(
            -4,
            format!("CLOB auth error (status {})", resp.status),
        ));
    }
    let mut creds: Credentials =
        serde_json::from_slice(&resp.body).map_err(|e| error(-4, format!("json: {e}")))?;
    creds.nonce = CLOB_AUTH_NONCE;
    Ok(creds)
}

pub fn clob_server_time() -> Result<u64, DispatchResponse> {
    let response = petal::sdk::http_fetch(
        &HttpRequest {
            method: "GET".into(),
            url: format!("{}/time", crate::runtime_config::clob_url()),
            headers: Vec::new(),
            body: Vec::new(),
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)?;
    if !(200..300).contains(&response.status) {
        return Err(error(
            -4,
            format!("CLOB time failed with status {}", response.status),
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|err| error(-4, format!("CLOB time JSON: {err}")))?;
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
        .or_else(|| value.get("timestamp").and_then(serde_json::Value::as_u64))
        .or_else(|| value.get("time").and_then(serde_json::Value::as_u64))
        .ok_or_else(|| error(-4, "CLOB time response is invalid"))
}
