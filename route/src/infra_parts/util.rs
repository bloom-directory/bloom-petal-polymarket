use crate::prelude::*;

use crate::polymarket::{PolymarketError, Result};
use petal::sdk::{DispatchResponse, HostStatus, SdkError};
use url::Url;
pub fn validate_relative_path(relative: &str) -> Result<&str, String> {
    if relative.is_empty() {
        return Ok(relative);
    }
    for segment in relative.split('/') {
        if !is_safe_segment(segment) {
            return Err(format!("invalid path segment '{segment}'"));
        }
    }
    Ok(relative)
}

pub fn url_with_query(base: &str, pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse(base).expect("hard-coded Polymarket URL must parse");
    for (key, value) in pairs {
        url.query_pairs_mut().append_pair(key, value);
    }
    url.to_string()
}

pub fn now_secs() -> u64 {
    petal::sdk::now_ms() / 1000
}

pub fn now_millis() -> u128 {
    u128::from(petal::sdk::now_ms())
}

pub fn sdk_error(e: SdkError) -> DispatchResponse {
    match e {
        SdkError::Host(HostStatus::NotFound) => error(-1, "not found"),
        SdkError::Host(HostStatus::Denied) => error(-2, "denied"),
        SdkError::Host(HostStatus::Invalid) => error(-3, "invalid"),
        SdkError::Host(HostStatus::Backend) => error(-4, "backend error"),
        SdkError::Host(HostStatus::BufferTooSmall { needed }) => {
            error(-5, format!("response too large: needs {needed} bytes"))
        }
        other => error(-4, other.message()),
    }
}

pub fn sdk_error_with_context(context: &str, e: SdkError) -> DispatchResponse {
    let code = match &e {
        SdkError::Host(HostStatus::NotFound) => -1,
        SdkError::Host(HostStatus::Denied) => -2,
        SdkError::Host(HostStatus::Invalid) => -3,
        SdkError::Host(HostStatus::Backend) => -4,
        SdkError::Host(HostStatus::BufferTooSmall { .. }) => -5,
        _ => -4,
    };
    error(code, format!("{context}: {}", e.message()))
}

pub fn polymarket_error(e: PolymarketError) -> DispatchResponse {
    match e {
        PolymarketError::Invalid(message) => error(-3, message),
        other => error(-4, other.to_string()),
    }
}
