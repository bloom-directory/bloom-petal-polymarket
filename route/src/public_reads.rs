use crate::prelude::*;

use crate::polymarket::{Market, Result};
use alloy::primitives::Address;
use petal::sdk::DispatchResponse;
pub fn market_by_slug(slug: &str) -> Result<Market, DispatchResponse> {
    get_json(&format!(
        "{}/markets/slug/{slug}",
        crate::runtime_config::gamma_url()
    ))
}

pub fn position_user(segment: &str) -> Result<String, DispatchResponse> {
    if (segment.starts_with("0x") || segment.starts_with("0X"))
        && let Ok(address) = segment.parse::<Address>()
    {
        return Ok(address.to_checksum(None));
    }
    wallet_address(segment).map(|address| address.to_checksum(None))
}
