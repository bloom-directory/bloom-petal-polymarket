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
pub(crate) const POLYMARKET_WEB: &str = "https://polymarket.com";
pub(crate) const RELAYER: &str = "https://relayer-v2.polymarket.com";
pub(crate) const CLOB_AUTH_NONCE: u32 = 0;
pub(crate) const ONBOARD_POLL_TIMEOUT_SECS: u64 = 180;
pub(crate) const ONBOARD_POLL_INTERVAL_SECS: u64 = 2;
pub(crate) const BATCH_DEADLINE_SECS: u64 = 3600;

pub(crate) const ROOT_DIRS: [&str; 8] = [
    "markets",
    "search",
    "positions",
    "onboard",
    "account",
    "trade",
    "fund",
    "meta",
];
pub(crate) const META_FILES: [&str; 1] = ["parity.json"];
pub(crate) const MARKET_FILES: [&str; 3] = ["market.json", "book.json", "prices.json"];
pub(crate) const POSITION_FILES: [&str; 3] = ["positions.json", "trades.json", "activity.json"];
pub(crate) const ONBOARD_FILES: [&str; 3] = ["status.json", "plan.md", "approvals.json"];
pub(crate) const ACCOUNT_FILES: [&str; 2] = ["portfolio.json", "orders.json"];
pub(crate) const FUND_FILES: [&str; 3] = ["plan.md", "request.json", "status.json"];
pub(crate) const DRAFT_FILES: [&str; 6] = [
    "plan.md",
    "order.json",
    "policy_check.json",
    "quote.json",
    "review_intent.json",
    "post_attempt.json",
];
pub(crate) const DRAFT_WRITABLE_FILES: [&str; 2] = ["revalidate", "post"];
pub(crate) const RECEIPT_FILES: [&str; 1] = ["receipt.json"];
pub(crate) const RECEIPT_WRITABLE_FILES: [&str; 1] = ["cancel"];

pub(crate) const BEGIN_HINT: &str =
    "write anything here to mint or derive CLOB credentials with the daemon keystore\n";
pub(crate) const TRADE_NEW_HINT: &str = r#"write JSON to create a reviewable draft, e.g.
{"slug":"will-canada-win-the-2026-fifa-world-cup-755","outcome":"yes","amount":"1","max_price":"0.01"}
"#;
pub(crate) const FUND_NEW_HINT: &str = r#"write JSON to create a reviewable pUSD funding request, e.g.
{"target_pusd":"10","max_spend":"100","from_token":"native","slippage_bps":50}
"#;
pub(crate) const TRADE_REVALIDATE_HINT: &str = r#"write {"revalidate":true} to revalidate this draft and stage the final review artifact. Revalidated drafts can then be posted by writing {"post":true} to post; resting GTC orders can be cancelled from their receipt.
"#;
pub(crate) const TRADE_POST_HINT: &str = r#"write {"post":true} to sign and post a revalidated draft, then write a private receipt. This performs a value-moving CLOB POST /order.
"#;
pub(crate) const TRADE_CANCEL_HINT: &str = r#"write {"cancel":true} to cancel the posted CLOB order recorded by this receipt. Cancelling uses CLOB DELETE /order and updates the private receipt/draft status.
"#;
