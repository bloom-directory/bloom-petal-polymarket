use std::collections::BTreeSet;

use crate::polymarket::order::{OrderType, parse_micro};
use crate::polymarket::trade as shared_trade;
use crate::polymarket::{Market, Side};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TradeNewRequest {
    pub(crate) slug: String,
    pub(crate) outcome: String,
    pub(crate) amount: String,
    #[serde(default)]
    pub(crate) side: Option<String>,
    #[serde(default)]
    pub(crate) max_price: Option<String>,
    #[serde(default)]
    pub(crate) min_price: Option<String>,
    #[serde(default)]
    pub(crate) limit_price: Option<String>,
    #[serde(default)]
    pub(crate) order_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct LocalWalletPolicy {
    #[serde(default)]
    pub(crate) polymarket: LocalPolymarketPolicy,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LocalPolymarketPolicy {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default, with = "local_micro_opt")]
    pub(crate) max_order_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    pub(crate) max_daily_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    pub(crate) require_flag_above_usd: Option<u64>,
    #[serde(default, with = "local_micro_opt")]
    pub(crate) max_price: Option<u64>,
    #[serde(default = "default_true")]
    pub(crate) allow_neg_risk: bool,
    #[serde(default)]
    pub(crate) allowed_slugs: BTreeSet<String>,
    #[serde(default)]
    pub(crate) denied_slugs: BTreeSet<String>,
    #[serde(default)]
    pub(crate) allowed_condition_ids: BTreeSet<String>,
    #[serde(default)]
    pub(crate) denied_condition_ids: BTreeSet<String>,
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

pub(crate) fn default_true() -> bool {
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
pub(crate) enum LocalPolicySide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalPolymarketOrderCtx {
    pub(crate) slug: String,
    pub(crate) condition_id: String,
    pub(crate) side: LocalPolicySide,
    pub(crate) amount_microusd: u64,
    pub(crate) limit_price_micro: u64,
    pub(crate) active: bool,
    pub(crate) closed: bool,
    pub(crate) order_book_enabled: bool,
    pub(crate) binary_outcomes: bool,
    pub(crate) neg_risk: bool,
    pub(crate) receipt_store_readable: bool,
    pub(crate) daily_posted_microusd: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalPolicyOutcome {
    Pass,
    Warn,
    Deny,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LocalPolicyCheck {
    pub(crate) rule: String,
    pub(crate) outcome: LocalPolicyOutcome,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TradeRevalidateRequest {
    pub(crate) revalidate: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TradePostRequest {
    pub(crate) post: bool,
    #[serde(default)]
    pub(crate) acknowledge_warnings: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TradeCancelRequest {
    pub(crate) cancel: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoreTradeDraft {
    pub(crate) id: String,
    pub(crate) wallet: String,
    pub(crate) slug: String,
    pub(crate) question: String,
    pub(crate) condition_id: String,
    pub(crate) outcome: String,
    pub(crate) token_id: String,
    pub(crate) side: Side,
    pub(crate) order_type: OrderType,
    pub(crate) amount_micro: u64,
    pub(crate) price_bound_micro: u64,
    pub(crate) limit_price: Option<String>,
    pub(crate) marketable: bool,
    pub(crate) limit_price_micro: u64,
    pub(crate) size_micro: u64,
    pub(crate) maker_micro: u64,
    pub(crate) taker_micro: u64,
    pub(crate) tick_micro: u64,
    pub(crate) min_order_size_micro: u64,
    pub(crate) neg_risk: bool,
    pub(crate) active: bool,
    pub(crate) closed: bool,
    pub(crate) order_book_enabled: bool,
    pub(crate) binary_outcomes: bool,
    pub(crate) best_ask_micro: Option<u64>,
    pub(crate) best_bid_micro: Option<u64>,
    pub(crate) book_snapshot_secs: u64,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) salt: Option<u64>,
    #[serde(default)]
    pub(crate) clob_order_id: Option<String>,
    #[serde(default)]
    pub(crate) clob_status: Option<String>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StoreTradeReceiptPolicy {
    pub(crate) side: Side,
    #[serde(default)]
    pub(crate) order_type: Option<OrderType>,
    pub(crate) amount_microusd: u64,
    pub(crate) clob_status: String,
    pub(crate) posted_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoreTradeReceipt {
    pub(crate) draft_id: String,
    pub(crate) wallet: String,
    pub(crate) slug: String,
    pub(crate) token_id: String,
    pub(crate) side: Side,
    pub(crate) order_type: OrderType,
    pub(crate) funder: Option<String>,
    pub(crate) signature_type: u8,
    pub(crate) amount_microusd: u64,
    pub(crate) limit_price_micro: u64,
    pub(crate) size_micro: u64,
    pub(crate) salt: u64,
    pub(crate) clob_order_id: Option<String>,
    pub(crate) clob_status: String,
    pub(crate) filled_size_micro: Option<u64>,
    pub(crate) raw_response: serde_json::Value,
    pub(crate) review_intent_hash: Option<String>,
    pub(crate) posted_ms: u128,
}

#[derive(Debug, Clone)]
pub(crate) struct TradeSnapshot {
    pub(crate) market: Market,
    pub(crate) outcome: String,
    pub(crate) token_id: String,
    pub(crate) neg_risk: bool,
    pub(crate) tick_micro: u64,
    pub(crate) min_size_micro: u64,
    pub(crate) best_ask_micro: Option<u64>,
    pub(crate) best_bid_micro: Option<u64>,
    pub(crate) active: bool,
    pub(crate) closed: bool,
    pub(crate) order_book_enabled: bool,
}

impl TradeSnapshot {
    pub(crate) fn as_shared(&self) -> shared_trade::Snapshot {
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
pub(crate) struct FundNewRequest {
    pub(crate) target_pusd: String,
    pub(crate) max_spend: String,
    #[serde(default)]
    pub(crate) from_token: Option<String>,
    #[serde(default = "default_slippage_bps")]
    pub(crate) slippage_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoreFundSession {
    pub(crate) id: String,
    pub(crate) wallet: String,
    pub(crate) target_pusd: String,
    pub(crate) max_spend: String,
    pub(crate) from_token: String,
    pub(crate) slippage_bps: u16,
    #[serde(default)]
    pub(crate) deposit_wallet: String,
    #[serde(default)]
    pub(crate) deposit_wallet_source: String,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) prepared_funding: Option<PreparedFunding>,
    #[serde(default)]
    pub(crate) review_intent: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) outbox_ids: Vec<String>,
    #[serde(default)]
    pub(crate) outbox_inspections: Vec<serde_json::Value>,
    #[serde(default)]
    pub(crate) next_transaction: usize,
    #[serde(default)]
    pub(crate) plan_md: Option<String>,
    #[serde(default)]
    pub(crate) approval: Option<serde_json::Value>,
    /// Set before calling the outbox stage host function and cleared only after
    /// its returned id is durable. A surviving marker means the call may have
    /// reached the host, so retries must fail closed instead of restaging.
    #[serde(default)]
    pub(crate) staging_transaction: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub(crate) struct FundConfirmRequest {
    #[serde(default)]
    pub(crate) confirm: bool,
    #[serde(default)]
    pub(crate) acknowledge_warnings: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedEvmTransaction {
    pub(crate) purpose: String,
    pub(crate) to: String,
    pub(crate) value_wei: String,
    pub(crate) data_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedFunding {
    pub(crate) review_intent: serde_json::Value,
    pub(crate) transactions: Vec<PreparedEvmTransaction>,
}

impl PreparedFunding {
    pub(crate) fn digest(&self) -> String {
        blake3::hash(&serde_json::to_vec(self).expect("prepared funding serializes"))
            .to_hex()
            .to_string()
    }
}

pub(crate) fn default_slippage_bps() -> u16 {
    50
}
