pub(crate) const MAX_HTTP_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_STORE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_LIST_BYTES: usize = 256 * 1024;
pub(crate) const MAX_POLICY_BYTES: usize = 256 * 1024;
pub(crate) const MAX_CHAIN_METHOD_BYTES: usize = 256 * 1024;
pub(crate) const MAX_CHAIN_READ_BYTES: usize = 256 * 1024;
pub(crate) const MARKETS_LIST_LIMIT: u32 = 20;
pub(crate) const TRADE_LOCK_STALE_MS: u128 = 5 * 60 * 1000;

pub(crate) const GAMMA: &str = "https://gamma-api.polymarket.com";
pub(crate) const DATA: &str = "https://data-api.polymarket.com";
pub(crate) const CLOB: &str = "https://clob.polymarket.com";
pub(crate) const RELAYER: &str = "https://relayer-v2.polymarket.com";
pub(crate) const CLOB_AUTH_NONCE: u32 = 0;
pub(crate) const ONBOARD_POLL_TIMEOUT_SECS: u64 = 180;
pub(crate) const ONBOARD_POLL_INTERVAL_SECS: u64 = 2;
pub(crate) const BATCH_DEADLINE_SECS: u64 = 3600;
