use std::collections::BTreeSet;

use crate::polymarket::order::{OrderType, parse_micro};
use crate::polymarket::trade as shared_trade;
use crate::polymarket::{Market, Side};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Deserialize)]
pub struct TradeNewRequest {
    pub slug: String,
    pub outcome: String,
    pub amount: String,
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub max_price: Option<String>,
    #[serde(default)]
    pub min_price: Option<String>,
    #[serde(default)]
    pub limit_price: Option<String>,
    #[serde(default)]
    pub order_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocalWalletPolicy {
    #[serde(default)]
    pub polymarket: LocalPolymarketPolicy,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalPolymarketPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, with = "local_micro_opt")]
    pub max_order_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    pub max_daily_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    pub require_flag_above_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    pub max_price: Option<u64>,
    #[serde(default = "default_true")]
    pub allow_neg_risk: bool,
    #[serde(default)]
    pub allowed_slugs: BTreeSet<String>,
    #[serde(default)]
    pub denied_slugs: BTreeSet<String>,
    #[serde(default)]
    pub allowed_condition_ids: BTreeSet<String>,
    #[serde(default)]
    pub denied_condition_ids: BTreeSet<String>,
}

impl Default for LocalPolymarketPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_order_usd: None,
            max_daily_usd: None,
            require_flag_above_usd: None,
            max_price: None,
            allow_neg_risk: true,
            allowed_slugs: BTreeSet::new(),
            denied_slugs: BTreeSet::new(),
            allowed_condition_ids: BTreeSet::new(),
            denied_condition_ids: BTreeSet::new(),
        }
    }
}

pub fn default_true() -> bool {
    true
}

mod local_micro_opt {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            S(String),
            I(i64),
            F(f64),
        }
        match Option::<Raw>::deserialize(d)? {
            None => Ok(None),
            Some(Raw::S(s)) => super::parse_micro(s.trim())
                .map(Some)
                .map_err(D::Error::custom),
            Some(Raw::I(i)) => {
                if i < 0 {
                    return Err(D::Error::custom("USD amount cannot be negative"));
                }
                (i as u64)
                    .checked_mul(1_000_000)
                    .map(Some)
                    .ok_or_else(|| D::Error::custom("USD amount too large"))
            }
            Some(Raw::F(f)) => super::parse_micro(&format!("{f}"))
                .map(Some)
                .map_err(D::Error::custom),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalPolicySide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct LocalPolymarketOrderCtx {
    pub slug: String,
    pub condition_id: String,
    pub side: LocalPolicySide,
    pub amount_microusd: u64,
    pub limit_price_micro: u64,
    pub active: bool,
    pub closed: bool,
    pub order_book_enabled: bool,
    pub binary_outcomes: bool,
    pub neg_risk: bool,
    pub receipt_store_readable: bool,
    pub daily_posted_microusd: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalPolicyOutcome {
    Pass,
    Warn,
    Deny,
}

#[derive(Debug, Clone, Serialize)]
pub struct LocalPolicyCheck {
    pub rule: String,
    pub outcome: LocalPolicyOutcome,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TradeRevalidateRequest {
    pub revalidate: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TradePostRequest {
    pub post: bool,
    #[serde(default)]
    pub acknowledge_warnings: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TradeCancelRequest {
    pub cancel: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreTradeDraft {
    pub id: String,
    pub wallet: String,
    pub slug: String,
    pub question: String,
    pub condition_id: String,
    pub outcome: String,
    pub token_id: String,
    pub side: Side,
    pub order_type: OrderType,
    pub amount_micro: u64,
    pub price_bound_micro: u64,
    pub limit_price: Option<String>,
    pub marketable: bool,
    pub limit_price_micro: u64,
    pub size_micro: u64,
    pub maker_micro: u64,
    pub taker_micro: u64,
    pub tick_micro: u64,
    pub min_order_size_micro: u64,
    pub neg_risk: bool,
    pub active: bool,
    pub closed: bool,
    pub order_book_enabled: bool,
    pub binary_outcomes: bool,
    pub best_ask_micro: Option<u64>,
    pub best_bid_micro: Option<u64>,
    pub book_snapshot_secs: u64,
    pub status: String,
    #[serde(default)]
    pub salt: Option<u64>,
    #[serde(default)]
    pub clob_order_id: Option<String>,
    #[serde(default)]
    pub clob_status: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StoreTradeReceiptPolicy {
    pub side: Side,
    #[serde(default)]
    pub order_type: Option<OrderType>,
    pub amount_microusd: u64,
    pub clob_status: String,
    pub posted_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreTradeReceipt {
    pub draft_id: String,
    pub wallet: String,
    pub slug: String,
    pub token_id: String,
    pub side: Side,
    pub order_type: OrderType,
    pub funder: Option<String>,
    pub signature_type: u8,
    pub amount_microusd: u64,
    pub limit_price_micro: u64,
    pub size_micro: u64,
    pub salt: u64,
    pub clob_order_id: Option<String>,
    pub clob_status: String,
    pub filled_size_micro: Option<u64>,
    pub raw_response: serde_json::Value,
    pub review_intent_hash: Option<String>,
    pub posted_ms: u128,
}

#[derive(Debug, Clone)]
pub struct TradeSnapshot {
    pub market: Market,
    pub outcome: String,
    pub token_id: String,
    pub neg_risk: bool,
    pub tick_micro: u64,
    pub min_size_micro: u64,
    pub best_ask_micro: Option<u64>,
    pub best_bid_micro: Option<u64>,
    pub active: bool,
    pub closed: bool,
    pub order_book_enabled: bool,
}

impl TradeSnapshot {
    pub fn as_shared(&self) -> shared_trade::Snapshot {
        shared_trade::Snapshot {
            market: self.market.clone(),
            token_id: self.token_id.clone(),
            neg_risk: self.neg_risk,
            tick_micro: self.tick_micro,
            min_size_micro: self.min_size_micro,
            best_ask_micro: self.best_ask_micro,
            best_bid_micro: self.best_bid_micro,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FundNewRequest {
    pub target_pusd: String,
    pub max_spend: String,
    #[serde(default)]
    pub from_token: Option<String>,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreFundSession {
    pub id: String,
    pub wallet: String,
    pub target_pusd: String,
    pub max_spend: String,
    pub from_token: String,
    pub slippage_bps: u16,
    #[serde(default)]
    pub deposit_wallet: String,
    #[serde(default)]
    pub deposit_wallet_source: String,
    pub status: String,
    #[serde(default)]
    pub prepared_funding: Option<PreparedFunding>,
    #[serde(default)]
    pub review_intent: Option<serde_json::Value>,
    #[serde(default)]
    pub outbox_ids: Vec<String>,
    #[serde(default)]
    pub outbox_inspections: Vec<serde_json::Value>,
    #[serde(default)]
    pub next_transaction: usize,
    #[serde(default)]
    pub plan_md: Option<String>,
    #[serde(default)]
    pub approval: Option<serde_json::Value>,
    /// Set before calling the outbox stage host function and cleared only after
    /// its returned id is durable. A surviving marker means the call may have
    /// reached the host, so retries must fail closed instead of restaging.
    #[serde(default)]
    pub staging_transaction: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct FundConfirmRequest {
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub acknowledge_warnings: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedEvmTransaction {
    pub purpose: String,
    pub to: String,
    pub value_wei: String,
    pub data_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedFunding {
    pub review_intent: serde_json::Value,
    pub transactions: Vec<PreparedEvmTransaction>,
}

impl PreparedFunding {
    pub fn digest(&self) -> String {
        blake3::hash(&serde_json::to_vec(self).expect("prepared funding serializes"))
            .to_hex()
            .to_string()
    }
}

pub fn default_slippage_bps() -> u16 {
    50
}
