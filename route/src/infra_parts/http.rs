use crate::*;

use crate::bloom_petal_sdk::{DispatchResponse, HttpRequest};
use crate::polymarket::{Credentials, Result};
pub(crate) fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, DispatchResponse> {
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

pub(crate) fn clob_auth_request(
    method: &str,
    path: &str,
    headers: &[(&str, String)],
) -> Result<Credentials, DispatchResponse> {
    let resp = bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: method.into(),
            url: format!("{CLOB}{path}"),
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
