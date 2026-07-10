pub(crate) mod builder_creds;
pub(crate) mod creds;
pub(crate) mod eip712;
pub(crate) mod order;
pub(crate) mod order_model;
#[cfg(test)]
pub(crate) mod order_store;
pub(crate) mod signer;
pub(crate) mod signing;
pub(crate) mod trade;
pub(crate) mod types;
pub(crate) mod wallet;

pub use builder_creds::BuilderCredentials;
pub use eip712::derive_deposit_wallet_address;
#[allow(unused_imports)]
pub use types::{Credentials, Market, OrderBook, Position, Side, Trade};

pub const POLYGON: u64 = 137;
pub const AMOY: u64 = 80_002;
pub const ACTION_ID_HEX_PREFIX: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum PolymarketError {
    #[error("url: {0}")]
    Url(#[from] url::ParseError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("polymarket api error (status {status}): {body}")]
    Api { status: u16, body: String },
    #[error("signing: {0}")]
    Signing(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

impl PolymarketError {
    pub fn signing(s: impl Into<String>) -> Self {
        PolymarketError::Signing(s.into())
    }

    pub fn invalid(s: impl Into<String>) -> Self {
        PolymarketError::Invalid(s.into())
    }
}

pub type Result<T, E = PolymarketError> = std::result::Result<T, E>;

pub fn validate_wallet_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(PolymarketError::invalid(format!(
            "invalid wallet name {name:?}: must be 1-64 chars of [A-Za-z0-9_-]"
        )));
    }
    Ok(())
}
