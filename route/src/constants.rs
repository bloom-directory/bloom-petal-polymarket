pub const MAX_HTTP_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_STORE_BYTES: usize = 1024 * 1024;
pub const MAX_LIST_BYTES: usize = 256 * 1024;
pub const MAX_POLICY_BYTES: usize = 256 * 1024;
pub const MAX_CHAIN_METHOD_BYTES: usize = 256 * 1024;
pub const MAX_CHAIN_READ_BYTES: usize = 256 * 1024;
pub const MARKETS_LIST_LIMIT: u32 = 20;
pub const TRADE_LOCK_STALE_MS: u128 = 5 * 60 * 1000;

pub const GAMMA: &str = "https://gamma-api.polymarket.com";
pub const DATA: &str = "https://data-api.polymarket.com";
pub const CLOB: &str = "https://clob.polymarket.com";
pub const RELAYER: &str = "https://relayer-v2.polymarket.com";
pub const CLOB_AUTH_NONCE: u32 = 0;
pub const ONBOARD_POLL_TIMEOUT_SECS: u64 = 180;
pub const ONBOARD_POLL_INTERVAL_SECS: u64 = 2;
pub const BATCH_DEADLINE_SECS: u64 = 3600;
