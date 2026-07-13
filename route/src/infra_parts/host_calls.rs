use crate::prelude::*;

use crate::polymarket::Result;
use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HttpRequest};
pub fn wallet_address(wallet: &str) -> Result<Address, DispatchResponse> {
    let path = format!("wallets/{wallet}/address");
    let bytes = petal::sdk::vfs_read(&path, 128).map_err(sdk_error)?;
    let raw = core::str::from_utf8(&bytes)
        .map_err(|e| error(-4, format!("wallet address is not utf-8: {e}")))?
        .trim();
    raw.parse::<Address>()
        .map_err(|e| error(-4, format!("wallet address parse: {e}")))
}

pub fn http(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> Result<petal::sdk::HttpResponse, DispatchResponse> {
    petal::sdk::http_fetch(
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
