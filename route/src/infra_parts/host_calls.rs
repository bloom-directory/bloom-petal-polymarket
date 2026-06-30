use crate::*;

use crate::bloom_petal_sdk::{DispatchResponse, HttpRequest};
use crate::polymarket::Result;
use alloy::primitives::Address;
pub(crate) fn wallet_address(wallet: &str) -> Result<Address, DispatchResponse> {
    let path = format!("wallets/{wallet}/address");
    let bytes = bloom_petal_sdk::vfs_read(&path, 128).map_err(sdk_error)?;
    let raw = core::str::from_utf8(&bytes)
        .map_err(|e| error(-4, format!("wallet address is not utf-8: {e}")))?
        .trim();
    raw.parse::<Address>()
        .map_err(|e| error(-4, format!("wallet address parse: {e}")))
}

pub(crate) fn http(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Result<bloom_petal_sdk::HttpResponse, DispatchResponse> {
    bloom_petal_sdk::http_fetch(
        &HttpRequest {
            method: method.into(),
            url: url.into(),
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into()))
                .collect(),
            body,
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)
}
